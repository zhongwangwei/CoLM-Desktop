use super::*;

#[test]
fn chunks_keep_the_inner_dimensions_whole_until_they_are_too_big() {
    // 小网格：一条记录整块。
    assert_eq!(chunk_shape(&[96, 16, 24]), [1, 16, 24]);
    assert_eq!(chunk_shape(&[96, 10, 16, 24]), [1, 10, 16, 24]);
    // 全球 0.1°：一层 1800×3600×8 = 52 MB，按纬度带切到 4 MiB 以内，经度保持完整。
    let global = chunk_shape(&[24, 1800, 3600]);
    assert_eq!(global, [1, 145, 3600]);
    assert!(global.iter().product::<usize>() * 8 <= TARGET_CHUNK_BYTES);
    // 有层维时先把层维收到 1。
    assert_eq!(chunk_shape(&[24, 10, 1800, 3600]), [1, 1, 145, 3600]);
    // 空的维度也给出合法的块（HDF5 要求块长至少 1）。
    assert_eq!(chunk_shape(&[0, 0, 5]), [1, 1, 5]);
}

#[test]
fn chunk_origins_walk_the_grid_in_row_major_order() {
    let grid = [2, 3];
    let chunk = [1, 4];
    let origins: Vec<_> = (0..6).map(|i| chunk_origin(i, &grid, &chunk)).collect();
    assert_eq!(
        origins,
        [
            vec![0, 0],
            vec![0, 4],
            vec![0, 8],
            vec![1, 0],
            vec![1, 4],
            vec![1, 8]
        ]
    );
}

#[test]
fn edge_chunks_are_padded_to_the_full_chunk() {
    // 形状 (1, 3, 5)，块 (1, 2, 4)：右下角那块只有 1×1 个真值。
    let data: Vec<f64> = (0..15).map(f64::from).collect();
    let bytes = chunk_bytes(&data, &[1, 3, 5], &[1, 2, 4], &[0, 2, 4]);
    let values: Vec<f64> = bytes
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(values.len(), 8);
    assert_eq!(values[0], 14.0);
    // 中间那块：第 0、1 行的第 4 列开始。
    let bytes = chunk_bytes(&data, &[1, 3, 5], &[1, 2, 4], &[0, 0, 4]);
    let values: Vec<f64> = bytes
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!((values[0], values[4]), (4.0, 9.0));
}

/// 用 netcdf crate 定义、直写块写数据、再用 netcdf crate 读回：值逐位相同，边缘块不完整也一样。
#[test]
fn values_read_back_bitwise_through_netcdf() {
    let dir = std::env::temp_dir().join(format!("colm-h5chunk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.nc");
    let shape = [5usize, 3, 7, 11];
    // 小目标块：逼出层维、纬度方向的切分和边缘块。
    let chunk = chunk_shape_for(&shape, 8 * 30);
    assert_eq!(chunk, [1, 1, 2, 11]);
    let data: Vec<f64> = (0..shape.iter().product::<usize>())
        .map(|i| (i as f64).sin() * 1e3 + if i % 17 == 0 { 1e36 } else { 0.0 })
        .collect();
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_unlimited_dimension("time").unwrap();
        file.add_dimension("layer", shape[1]).unwrap();
        file.add_dimension("lat", shape[2]).unwrap();
        file.add_dimension("lon", shape[3]).unwrap();
        let mut variable = file
            .add_variable::<f64>("f_x", &["time", "layer", "lat", "lon"])
            .unwrap();
        variable.set_chunking(&chunk).unwrap();
        variable.set_compression(1, false).unwrap();
        variable.put_attribute("missing_value", 1e36).unwrap();
        file.close().unwrap();
    }
    let mut writer = ChunkWriter::open(&path).unwrap();
    writer.write_f64("f_x", &shape, &chunk, &data, 1).unwrap();
    writer.close().unwrap();

    let file = netcdf::open(&path).unwrap();
    assert_eq!(file.dimension("time").unwrap().len(), shape[0]);
    let variable = file.variable("f_x").unwrap();
    let read: Vec<f64> = variable.get_values(netcdf::Extents::All).unwrap();
    assert_eq!(read.len(), data.len());
    assert!(read
        .iter()
        .zip(&data)
        .all(|(a, b)| a.to_bits() == b.to_bits()));
    drop(file);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mismatched_inputs_are_errors() {
    let dir = std::env::temp_dir().join(format!("colm-h5chunk-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_unlimited_dimension("time").unwrap();
        file.add_dimension("lon", 4).unwrap();
        let mut variable = file.add_variable::<f64>("f_x", &["time", "lon"]).unwrap();
        variable.set_chunking(&[1, 4]).unwrap();
        variable.set_compression(1, false).unwrap();
        file.close().unwrap();
    }
    let mut writer = ChunkWriter::open(&path).unwrap();
    assert!(writer
        .write_f64("f_x", &[2, 4], &[1, 4], &[0.0; 7], 1)
        .is_err());
    assert!(writer
        .write_f64("f_x", &[2, 4], &[1], &[0.0; 8], 1)
        .is_err());
    assert!(writer
        .write_f64("f_y", &[2, 4], &[1, 4], &[0.0; 8], 1)
        .is_err());
    writer.close().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 写盘速度对比（不进默认测试）：同一份数据、同样的块与 deflate 1，netcdf 串行 `put_values` 对并行直写块。
/// `COLM_H5CHUNK_BENCH_DIR` 指定写到哪里；`cargo test --release -p colm-h5chunk bench -- --ignored --nocapture`。
#[test]
#[ignore]
fn bench_parallel_chunks_against_put_values() {
    let dir = std::path::PathBuf::from(
        std::env::var("COLM_H5CHUNK_BENCH_DIR")
            .unwrap_or_else(|_| std::env::temp_dir().display().to_string()),
    );
    // 全球 0.25°，24 条记录，4 个量：每个量 199 MB，共 796 MB。平滑场加少量噪声、陆地外是缺测，
    // 压缩比与真实输出同一量级。
    let (records, nlat, nlon) = (24usize, 720usize, 1440usize);
    let shape = [records, nlat, nlon];
    let chunk = chunk_shape(&shape);
    let fields: Vec<Vec<f64>> = (0..4)
        .map(|v| {
            (0..records * nlat * nlon)
                .map(|i| {
                    let (t, y, x) = (i / (nlat * nlon), (i / nlon) % nlat, i % nlon);
                    if (x * 7 + y * 3) % 10 < 3 {
                        1e36
                    } else {
                        280.0
                            + 10.0
                                * ((y as f64) / 40.0 + v as f64).sin()
                                * ((x as f64) / 60.0).cos()
                            + t as f64 * 0.1
                            + ((i * 2654435761) % 1000) as f64 * 1e-4
                    }
                })
                .collect()
        })
        .collect();
    let raw = (fields.len() * records * nlat * nlon * 8) as f64 / 1e6;
    let define = |path: &std::path::Path| {
        let mut file = netcdf::create(path).unwrap();
        file.add_unlimited_dimension("time").unwrap();
        file.add_dimension("lat", nlat).unwrap();
        file.add_dimension("lon", nlon).unwrap();
        for v in 0..fields.len() {
            let mut variable = file
                .add_variable::<f64>(&format!("f_{v}"), &["time", "lat", "lon"])
                .unwrap();
            variable.set_chunking(&chunk).unwrap();
            variable.set_compression(1, false).unwrap();
        }
        file
    };
    let serial = dir.join("bench_put_values.nc");
    let start = std::time::Instant::now();
    let mut file = define(&serial);
    for (v, data) in fields.iter().enumerate() {
        file.variable_mut(&format!("f_{v}"))
            .unwrap()
            .put_values(data, [0..records, 0..nlat, 0..nlon])
            .unwrap();
    }
    file.close().unwrap();
    let serial_secs = start.elapsed().as_secs_f64();

    let parallel = dir.join("bench_chunks.nc");
    let start = std::time::Instant::now();
    define(&parallel).close().unwrap();
    let mut writer = ChunkWriter::open(&parallel).unwrap();
    // 与 colm-hist 的写法相同：几个量同时压缩，再串行写盘。
    let packed: Vec<PackedVariable> = fields
        .par_iter()
        .enumerate()
        .map(|(v, data)| pack_f64(&format!("f_{v}"), &shape, &chunk, data, 1).unwrap())
        .collect();
    for variable in &packed {
        writer.write(variable).unwrap();
    }
    writer.close().unwrap();
    let parallel_secs = start.elapsed().as_secs_f64();

    let size = |p: &std::path::Path| std::fs::metadata(p).unwrap().len() as f64 / 1e6;
    eprintln!(
        "{raw:.0} MB raw, chunk {chunk:?}, {} rayon threads\n  put_values (serial deflate): {serial_secs:6.2} s  {:6.0} MB/s  {:6.0} MB file\n  parallel chunks:             {parallel_secs:6.2} s  {:6.0} MB/s  {:6.0} MB file\n  speed-up {:.1}x",
        rayon::current_num_threads(),
        raw / serial_secs,
        size(&serial),
        raw / parallel_secs,
        size(&parallel),
        serial_secs / parallel_secs
    );
    let a = netcdf::open(&serial).unwrap();
    let b = netcdf::open(&parallel).unwrap();
    for v in 0..fields.len() {
        let name = format!("f_{v}");
        let x: Vec<f64> = a
            .variable(&name)
            .unwrap()
            .get_values(netcdf::Extents::All)
            .unwrap();
        let y: Vec<f64> = b
            .variable(&name)
            .unwrap()
            .get_values(netcdf::Extents::All)
            .unwrap();
        assert!(x.iter().zip(&y).all(|(x, y)| x.to_bits() == y.to_bits()));
    }
    drop((a, b));
    std::fs::remove_file(&serial).unwrap();
    std::fs::remove_file(&parallel).unwrap();
}
