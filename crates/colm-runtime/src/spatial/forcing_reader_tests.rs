//! 预读读取器：顺序、回跳、跨文件读都与直接读逐位相同。

use super::*;

/// 写一个 `(time, lat, lon)` 的 `f32` 文件，值由 `seed` 决定（`f32` 读成 `f64` 走库里的类型转换）。
fn write(path: &Path, records: usize, seed: f32) {
    let mut file = netcdf::create(path).unwrap();
    file.add_unlimited_dimension("time").unwrap();
    file.add_dimension("lat", 3).unwrap();
    file.add_dimension("lon", 4).unwrap();
    let mut variable = file
        .add_variable::<f32>("tmp", &["time", "lat", "lon"])
        .unwrap();
    variable.set_chunking(&[1, 3, 4]).unwrap();
    variable.set_compression(1, true).unwrap();
    let values: Vec<f32> = (0..records * 12)
        .map(|i| seed + (i as f32) * 0.1_f32.sqrt())
        .collect();
    variable
        .put_values(&values, (0..records, 0..3, 0..4))
        .unwrap();
}

fn direct(path: &Path, record: usize, window: Window) -> Vec<f64> {
    let file = netcdf::open(path).unwrap();
    let ((lat0, lat1), (lon0, lon1)) = window;
    file.variable("tmp")
        .unwrap()
        .get_values((record - 1, lat0..=lat1, lon0..=lon1))
        .unwrap()
}

#[test]
fn prefetched_records_match_direct_reads_bit_for_bit() {
    let dir = std::env::temp_dir().join(format!("colm-forcing-reader-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (first, second) = (dir.join("a.nc"), dir.join("b.nc"));
    write(&first, 5, 1.25);
    write(&second, 4, -3.5);
    let window = ((1, 2), (0, 2));
    let reader = RecordReader::new();
    // 顺序读到文件末尾（最后一次预读落在末尾之后），换文件，再回跳。
    let plan = [
        (&first, 1),
        (&first, 2),
        (&first, 3),
        (&first, 4),
        (&first, 5),
        (&second, 1),
        (&second, 2),
        (&first, 2),
        (&second, 4),
    ];
    for (path, record) in plan {
        let got = reader.read_block(path, "tmp", record, window).unwrap();
        let want = direct(path, record, window);
        assert_eq!(
            got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            want.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "{} record {record}",
            path.display()
        );
    }
    // 读不存在的变量照样报错，不会卡住。
    assert!(reader.read_block(&first, "missing", 1, window).is_err());
    drop(reader);
    std::fs::remove_dir_all(&dir).unwrap();
}
