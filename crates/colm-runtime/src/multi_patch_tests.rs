use super::*;

fn source(dir: &std::path::Path) -> RestartFile {
    let path = dir.join("restart.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", 2).unwrap();
    file.add_dimension("pft", 2).unwrap();
    file.add_dimension("soil", 3).unwrap();
    file.add_variable::<f64>("zwt", &["patch"])
        .unwrap()
        .put_values(&[1.0, 2.0], ..)
        .unwrap();
    file.add_variable::<f64>("wliq", &["patch", "soil"])
        .unwrap()
        .put_values(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], ..)
        .unwrap();
    file.add_variable::<i32>("cphase_p", &["pft"])
        .unwrap()
        .put_values(&[0, 0], ..)
        .unwrap();
    file.add_variable::<f64>("pondmx", &[])
        .unwrap()
        .put_value(10.0, ..)
        .unwrap();
    drop(file);
    RestartFile::open(&path).unwrap()
}

/// 整变量写法（只换自己那块）与"只有自己那块"的写法都能拼回去；无 patch 维的量取第一个。
#[test]
fn patch_blocks_are_stitched_from_each_patch() {
    let dir = std::env::temp_dir().join(format!("colm-multi-patch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = source(&dir);
    let slots = [
        PatchSlot {
            patch: 0,
            pfts: 0..1,
        },
        PatchSlot {
            patch: 1,
            pfts: 1..2,
        },
    ];
    let merged = merge_overrides(
        &source,
        &slots,
        vec![
            vec![
                RestartOverride::new("zwt", vec![10.0, 2.0]),
                RestartOverride::new("wliq", vec![7.0, 8.0, 9.0]),
                RestartOverride::new("cphase_p", vec![3.0]),
                RestartOverride::new("pondmx", vec![10.0]),
            ],
            vec![
                RestartOverride::new("zwt", vec![1.0, 20.0]),
                RestartOverride::new("wliq", vec![1.0, 2.0, 3.0, 10.0, 11.0, 12.0]),
                RestartOverride::new("cphase_p", vec![4.0]),
                RestartOverride::new("pondmx", vec![99.0]),
            ],
        ],
    )
    .unwrap();
    let get = |name: &str| {
        merged
            .iter()
            .find(|over| over.name == name)
            .unwrap()
            .values
            .clone()
    };
    assert_eq!(get("zwt"), vec![10.0, 20.0]);
    assert_eq!(get("wliq"), vec![7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
    assert_eq!(get("cphase_p"), vec![3.0, 4.0]);
    assert_eq!(get("pondmx"), vec![10.0]);
    let _ = std::fs::remove_dir_all(&dir);
}
