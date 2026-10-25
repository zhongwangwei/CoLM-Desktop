use super::*;

#[test]
fn block_names_decode_to_their_south_west_corner() {
    assert_eq!(block_origin("e110_n20").unwrap(), (110.0, 20.0));
    assert_eq!(block_origin("w180_s90").unwrap(), (-180.0, -90.0));
    assert!(block_origin("x110_n20").is_err());
    assert!(block_origin("e110").is_err());
}

/// 上游 `block_set_local_blocks`：外层经度块、内层纬度块，都从西南往东北。
#[test]
fn blocks_sort_longitude_first_then_latitude() {
    let mut names = ["e115_n20", "e110_n25", "w005_s10", "e110_n20"]
        .map(|name| (block_origin(name).unwrap(), name));
    names.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let order = names.map(|(_, name)| name);
    assert_eq!(order, ["w005_s10", "e110_n20", "e110_n25", "e115_n20"]);
}

/// CROP 的农田 patch 共用同一像元区间（同 `eindex`、同 `ipxstt`），只有这时才读 `pctshared`。
#[test]
fn shared_pixels_need_same_element_and_same_start() {
    assert!(!has_shared(&[1, 1, 2], &[1, 3, 1]));
    assert!(!has_shared(&[1, 2], &[1, 1]));
    assert!(has_shared(&[1, 1, 1, 2], &[1, 3, 3, 1]));
    assert!(!has_shared(&[7], &[1]));
}

use std::path::PathBuf;

fn scratch_directory(tag: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("colm-{tag}-{}-{unique}", std::process::id()))
}

#[test]
fn block_names_are_listed_in_upstream_order_without_opening_files() {
    let root = scratch_directory("block-names");
    let directory = root.join("landpatch").join("2005");
    std::fs::create_dir_all(&directory).unwrap();
    for block in ["e115_n20", "e110_n25", "w005_s10", "e110_n20"] {
        // 空文件就够：block_names 只解析名字，不打开 NetCDF。
        std::fs::write(directory.join(format!("landpatch_{block}.nc")), b"").unwrap();
    }
    // 不以 landpatch_ 开头的文件不是块。
    std::fs::write(directory.join("landpatch_bad_extra.txt"), b"").unwrap();
    let names = block_names(&root, 2005).unwrap();
    assert_eq!(names, ["w005_s10", "e110_n20", "e110_n25", "e115_n20"]);

    // 名字不合规的块文件必须报错，不能静默跳过。
    std::fs::write(directory.join("landpatch_not-a-block.nc"), b"").unwrap();
    assert!(block_names(&root, 2005).is_err());

    let _ = std::fs::remove_dir_all(&root);
}

/// 只读一个块时得到的向量，必须与全量读里对应块的切片逐元素相同；块的相对次序也要一致。
///
/// 依赖真实 landdata：`COLM_SPATIAL_FIXTURE=<case>/out/<case>/landdata`。仓库不自带空间 fixture。
#[test]
#[ignore = "needs COLM_SPATIAL_FIXTURE=<case>/out/<case>/landdata"]
fn selected_blocks_read_the_same_vectors_as_the_full_topology() {
    let source = PathBuf::from(
        std::env::var_os("COLM_SPATIAL_FIXTURE")
            .expect("set COLM_SPATIAL_FIXTURE to a case's landdata directory"),
    );
    let landpatch_root = source.join("landpatch");
    let year = std::fs::read_dir(&landpatch_root)
        .expect("read landpatch root")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
        .next()
        .expect("at least one <year> directory under landpatch");
    let year_directory = landpatch_root.join(format!("{year:04}"));
    let source_block = std::fs::read_dir(&year_directory)
        .expect("read year directory")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_prefix("landpatch_")
                .and_then(|rest| rest.strip_suffix(".nc"))
                .map(str::to_string)
        })
        .next()
        .expect("at least one landpatch block file");
    // 造两份几何相同的块：只验证“子集读”的切片与次序，不假装是真实网格。
    let second = if source_block == "w170_s80" {
        "e000_n00".to_owned()
    } else {
        "w170_s80".to_owned()
    };
    let root = scratch_directory("topology-subset");
    let landdata = root.join("landdata");
    let block_directory = landdata.join("landpatch").join(format!("{year:04}"));
    let mesh_directory = landdata.join("mesh").join(format!("{year:04}"));
    std::fs::create_dir_all(&block_directory).unwrap();
    std::fs::create_dir_all(&mesh_directory).unwrap();
    std::fs::copy(source.join("pixel.nc"), landdata.join("pixel.nc")).unwrap();
    let source_mesh = std::fs::read_dir(source.join("mesh").join(format!("{year:04}")))
        .expect("read mesh year directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("mesh_"))
        })
        .expect("a mesh block file");
    for block in [&second, &source_block] {
        std::fs::copy(
            year_directory.join(format!("landpatch_{source_block}.nc")),
            block_directory.join(format!("landpatch_{block}.nc")),
        )
        .unwrap();
        std::fs::copy(
            &source_mesh,
            mesh_directory.join(format!("mesh_{block}.nc")),
        )
        .unwrap();
    }

    let full = SpatialTopology::read(&landdata, year).unwrap();
    let names = block_names(&landdata, year).unwrap();
    assert_eq!(full.blocks.len(), 2);
    assert_eq!(names.len(), 2);
    for (index, name) in names.iter().enumerate() {
        let subset =
            SpatialTopology::read_selected(&landdata, year, Some(std::slice::from_ref(name)))
                .unwrap();
        let range = full.blocks[index].1.clone();
        assert_eq!(subset.blocks.len(), 1);
        assert_eq!(subset.blocks[0].0, *name);
        assert_eq!(subset.patch_count(), range.len());
        assert_eq!(subset.pixel, full.pixel);
        assert_eq!(subset.element, full.element[range.clone()]);
        assert_eq!(subset.land_type, full.land_type[range.clone()]);
        assert_eq!(subset.cells, full.cells[range.clone()]);
        assert_eq!(subset.shared_fraction, full.shared_fraction[range.clone()]);
    }
    // 未知块、重复选择都必须报错，不能静默少读。
    assert!(
        SpatialTopology::read_selected(&landdata, year, Some(&["e999_n99".to_owned()])).is_err()
    );
    assert!(SpatialTopology::read_selected(
        &landdata,
        year,
        Some(&[names[0].clone(), names[0].clone()])
    )
    .is_err());
    let _ = std::fs::remove_dir_all(&root);
}
