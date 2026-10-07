use super::*;

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-h5chunk-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{label}.nc"));
    let _ = std::fs::remove_file(&path);
    path
}

fn extents(start: &[usize], count: &[usize]) -> Vec<netcdf::Extent> {
    start
        .iter()
        .zip(count)
        .map(|(&start, &count)| netcdf::Extent::SliceCount {
            start,
            count,
            stride: 1,
        })
        .collect()
}

/// 几个区域：整片、跨块边界、贴着右下角的边缘块、单个元素、单行。
const REGIONS: [([usize; 3], [usize; 3]); 5] = [
    ([0, 0, 0], [3, 37, 53]),
    ([1, 5, 9], [2, 20, 30]),
    ([2, 30, 40], [1, 7, 13]),
    ([0, 36, 52], [1, 1, 1]),
    ([1, 11, 0], [1, 1, 53]),
];

/// 用 netCDF 照常写出、再用快路径与 `get_values` 各读一遍，比较位型。
fn check<F, T>(label: &str, values: &[F], chunk: &[usize], deflate: bool, shuffle: bool)
where
    F: netcdf::NcTypeDescriptor + Copy,
    T: Element + PartialEq + std::fmt::Debug,
{
    let path = scratch(label);
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 3).unwrap();
        file.add_dimension("lat", 37).unwrap();
        file.add_dimension("lon", 53).unwrap();
        let mut variable = file
            .add_variable::<F>("v", &["time", "lat", "lon"])
            .unwrap();
        variable.set_chunking(chunk).unwrap();
        if deflate {
            variable.set_compression(3, shuffle).unwrap();
        }
        variable.put_values(values, ..).unwrap();
    }
    let file = netcdf::open(&path).unwrap();
    let source = file.variable("v").unwrap();
    for (start, count) in REGIONS {
        let expected: Vec<T> = source.get_values(extents(&start, &count)).unwrap();
        let fast: Vec<T> = read_region_fast(&path, "v", &start, &count)
            .unwrap()
            .unwrap_or_else(|| panic!("{label}: fast path declined {start:?}+{count:?}"));
        assert_eq!(fast, expected, "{label} {start:?}+{count:?}");
        let via: Vec<T> = read_region(&path, "v", &start, &count).unwrap();
        assert_eq!(via, expected);
    }
}

fn wave(n: usize) -> impl Iterator<Item = f64> {
    (0..n).map(|i| (i as f64 * 0.37).sin() * 1000.0 + if i % 97 == 0 { 1e30 } else { 0.0 })
}

const N: usize = 3 * 37 * 53;

#[test]
fn f32_with_shuffle_and_deflate_reads_as_f64_bitwise() {
    let values: Vec<f32> = wave(N).map(|v| v as f32).collect();
    check::<f32, f64>("f32-shuffle", &values, &[1, 10, 16], true, true);
    check::<f32, f32>("f32-shuffle-same", &values, &[2, 7, 53], true, true);
}

#[test]
fn f64_deflate_without_shuffle_and_uncompressed_chunks() {
    let values: Vec<f64> = wave(N).collect();
    check::<f64, f64>("f64-deflate", &values, &[1, 37, 20], true, false);
    check::<f64, f64>("f64-plain-chunked", &values, &[3, 5, 5], false, false);
}

#[test]
fn integer_widening_matches_netcdf() {
    let shorts: Vec<i16> = (0..N)
        .map(|i| (i as i32 * 37 % 65536 - 32768) as i16)
        .collect();
    check::<i16, f64>("i16-f64", &shorts, &[1, 8, 8], true, true);
    check::<i16, i32>("i16-i32", &shorts, &[1, 8, 8], true, false);
    let bytes: Vec<u8> = (0..N).map(|i| (i * 7 % 256) as u8).collect();
    check::<u8, i32>("u8-i32", &bytes, &[1, 37, 53], true, true);
    let ints: Vec<i32> = (0..N).map(|i| i as i32 * -7919).collect();
    check::<i32, i32>("i32", &ints, &[1, 13, 17], true, true);
    check::<i32, i64>("i32-i64", &ints, &[1, 13, 17], true, true);
}

#[test]
fn unsupported_cases_fall_back_with_identical_results() {
    // 连续存储（不分块）。
    let path = scratch("contiguous");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("lat", 4).unwrap();
        file.add_dimension("lon", 5).unwrap();
        let mut variable = file.add_variable::<f64>("v", &["lat", "lon"]).unwrap();
        variable
            .put_values(&wave(20).collect::<Vec<_>>(), ..)
            .unwrap();
    }
    assert!(read_region_fast::<f64>(&path, "v", &[1, 1], &[2, 3])
        .unwrap()
        .is_none());
    let expected: Vec<f64> = netcdf::open(&path)
        .unwrap()
        .variable("v")
        .unwrap()
        .get_values(extents(&[1, 1], &[2, 3]))
        .unwrap();
    assert_eq!(
        read_region::<f64>(&path, "v", &[1, 1], &[2, 3]).unwrap(),
        expected
    );

    // 只写了一部分的块：没写的块未分配，读出来应是填充值 —— 交给 netCDF。
    let path = scratch("partial");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("lat", 8).unwrap();
        file.add_dimension("lon", 8).unwrap();
        let mut variable = file.add_variable::<f64>("v", &["lat", "lon"]).unwrap();
        variable.set_chunking(&[4, 4]).unwrap();
        variable.set_compression(1, false).unwrap();
        variable.put_values(&[1.0; 16], (0..4, 0..4)).unwrap();
    }
    assert!(read_region_fast::<f64>(&path, "v", &[0, 0], &[8, 8])
        .unwrap()
        .is_none());
    let via = read_region::<f64>(&path, "v", &[0, 0], &[8, 8]).unwrap();
    assert_eq!(via[0], 1.0);
    assert_eq!(via[63].to_bits(), 9.969_209_968_386_869e36_f64.to_bits());

    // 可能越界的转换（f64 → f32）不走快路径。
    let path = scratch("narrowing");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("lon", 4).unwrap();
        let mut variable = file.add_variable::<f64>("v", &["lon"]).unwrap();
        variable.set_chunking(&[4]).unwrap();
        variable.set_compression(1, false).unwrap();
        variable.put_values(&[1.0, 2.0, 3.0, 4.0], ..).unwrap();
    }
    assert!(read_region_fast::<f32>(&path, "v", &[0], &[4])
        .unwrap()
        .is_none());

    // 不存在的变量、不是 HDF5 的文件：快路径放弃，回落路径照常报错或读出。
    assert!(read_region_fast::<f64>(&path, "missing", &[0], &[4])
        .unwrap()
        .is_none());
    assert!(read_region::<f64>(&path, "missing", &[0], &[4]).is_err());
    let classic = scratch("classic");
    {
        let mut file = netcdf::create_with(&classic, netcdf::Options::_64BIT_OFFSET).unwrap();
        file.add_dimension("lon", 3).unwrap();
        file.add_variable::<f64>("v", &["lon"]).unwrap();
        file.enddef().unwrap();
        file.variable_mut("v")
            .unwrap()
            .put_values(&[5.0, 6.0, 7.0], ..)
            .unwrap();
    }
    assert!(read_region_fast::<f64>(&classic, "v", &[0], &[3])
        .unwrap()
        .is_none());
    assert_eq!(
        read_region::<f64>(&classic, "v", &[1], &[2]).unwrap(),
        [6.0, 7.0]
    );
}

/// 真实原始数据上对比快路径与 `get_values`（不进默认测试）。
/// `COLM_H5CHUNK_RAW=path:variable:start,..:count,.. cargo test -p colm-h5chunk real -- --ignored --nocapture`
#[test]
#[ignore]
fn real_raw_data_matches_get_values() {
    let spec = std::env::var("COLM_H5CHUNK_RAW").expect("set COLM_H5CHUNK_RAW");
    for item in spec.split(';') {
        let parts: Vec<&str> = item.split(':').collect();
        let parse = |s: &str| {
            s.split(',')
                .map(|v| v.parse().unwrap())
                .collect::<Vec<usize>>()
        };
        let (path, variable, start, count) = (
            std::path::Path::new(parts[0]),
            parts[1],
            parse(parts[2]),
            parse(parts[3]),
        );
        let started = std::time::Instant::now();
        let expected: Vec<f64> = netcdf::open(path)
            .unwrap()
            .variable(variable)
            .unwrap()
            .get_values(extents(&start, &count))
            .unwrap();
        let netcdf_secs = started.elapsed().as_secs_f64();
        let started = std::time::Instant::now();
        let fast = read_region_fast::<f64>(path, variable, &start, &count)
            .unwrap()
            .expect("fast path declined");
        let fast_secs = started.elapsed().as_secs_f64();
        assert!(fast
            .iter()
            .zip(&expected)
            .all(|(a, b)| a.to_bits() == b.to_bits()));
        eprintln!(
            "{} {variable} {count:?}: get_values {netcdf_secs:.2} s, parallel {fast_secs:.2} s, {} values bitwise equal",
            path.display(),
            fast.len()
        );
    }
}
