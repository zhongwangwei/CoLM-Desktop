//! 0.1° 网格 history 逐条写的吞吐探针（Phase 4 用，不是生产入口）。
//!
//! 用真实的 `HistoryBuffers::write_gridded_record` 写一个 0.1° 全球尺寸的网格
//! （默认 1800×3600 = 6.48M 格），量一条记录从聚合到落盘的墙钟，再把 5 个标量变量
//! 外推到 221 个变量的单条记录。环境变量：
//!
//! - `PROBE_LAT` / `PROBE_LON`：网格格数（默认 1800 / 3600）
//! - `PROBE_RECORDS`：写几条记录（默认 2；第 1 条建文件，后续追加）
//! - `PROBE_COMPRESS`：history 压缩级别（默认 1）
//! - `PROBE_DIR`：输出目录（默认 `/tmp`；tianhe 用 HDD_POOL）
//!
//! 运行：`cargo run --release -p colm-hist --features io --example gridded_stream_probe`

use std::sync::Arc;

use anyhow::{Context, Result};
use colm_hist::history::{HistoryBuffers, HistoryDimensions, HistoryGrid, HistorySite};

fn env_usize(name: &str, default: usize) -> Result<usize> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a whole number")),
        Err(_) => Ok(default),
    }
}

fn main() -> Result<()> {
    let nlat = env_usize("PROBE_LAT", 1800)?;
    let nlon = env_usize("PROBE_LON", 3600)?;
    let records = env_usize("PROBE_RECORDS", 2)?.max(1);
    let compress = env_usize("PROBE_COMPRESS", 1)?;
    anyhow::ensure!((0..=9).contains(&compress), "PROBE_COMPRESS must be 0..=9");
    let dir = std::env::var("PROBE_DIR").unwrap_or_else(|_| "/tmp".to_owned());
    let cells = nlat * nlon;
    let path =
        std::path::Path::new(&dir).join(format!("colm_hist_probe_{nlat}x{nlon}_c{compress}.nc"));
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {dir}"))?;
    let _ = std::fs::remove_file(&path);

    let latitude = (0..nlat)
        .map(|index| -90.0 + 180.0 * (index as f64 + 0.5) / nlat as f64)
        .collect::<Vec<_>>();
    let longitude = (0..nlon)
        .map(|index| 360.0 * (index as f64 + 0.5) / nlon as f64)
        .collect::<Vec<_>>();
    let grid = HistoryGrid {
        lat: latitude.clone(),
        lon: longitude.clone(),
        lat_s: latitude.iter().map(|lat| lat - 0.05).collect(),
        lat_n: latitude.iter().map(|lat| lat + 0.05).collect(),
        lon_w: longitude.iter().map(|lon| lon - 0.05).collect(),
        lon_e: longitude.iter().map(|lon| lon + 0.05).collect(),
        // 每个格一个 patch（≈0.1° 全球的 patch 数），值随 patch 变化：避免整场常数
        // 被压缩到只有几 MB，写吞吐与聚合成本都接近真实全球场。
        parts: (0..cells).map(|cell| vec![(cell, 1.0)]).collect(),
        patch_area: vec![1.0; cells],
        statics: Vec::new(),
        first_record_statics: Vec::new(),
        compress_level: u8::try_from(compress)?,
    };
    let dimensions = HistoryDimensions {
        patch: cells,
        soil: 10,
        lake: 10,
        snow_layers: 5,
        vegnodes: 4,
        band: 2,
        radiation_types: 2,
        sensor: 1,
    };
    let site = HistorySite {
        latitude_degrees: 0.0,
        longitude_degrees: 0.0,
    };
    let all_names = [
        "t_grnd", "fsena", "fevpa", "tref", "qref", "assim", "assimsha", "assimsun", "emis", "etr",
        "etrsha", "etrsun", "fevpg", "fevpl", "fgrnd", "fh", "fm", "fm10m", "fq", "frcsat",
        "fseng", "fsenl", "fsno", "green", "gssha", "gssun", "lai", "laisha", "laisun", "ldew",
        "lfevpa", "olrg", "qdrip", "qinfl", "qintr", "qstar", "respc", "rib", "rnet", "rnof",
    ];
    let nvars = env_usize("PROBE_VARS", 5)?;
    anyhow::ensure!(
        (1..=all_names.len()).contains(&nvars),
        "PROBE_VARS must be 1..={}",
        all_names.len()
    );
    let names = &all_names[..nvars];
    let mut buffers = HistoryBuffers::new(dimensions, site, 1)
        .with_grid(Arc::new(grid))
        .context("cannot attach the synthetic grid")?;
    buffers.declare(names)?;
    let wave = |row: usize| 280.0 + 0.05 * (row as f64) + 0.5 * ((row % 7) as f64);

    println!(
        "probe grid={nlat}x{nlon} cells={cells} records={records} compress={compress} vars={} path={}",
        names.len(),
        path.display()
    );
    // 填值只做一次：这里量的是 writer（聚合+压缩+落盘），不是模型每步更新槽位。
    for patch in 0..cells {
        buffers.select_patch(Some(patch))?;
        for name in names {
            buffers.set_patch_scalar(name, 0, wave(patch))?;
            buffers.include(name, 0)?;
        }
    }
    buffers.select_patch(None)?;
    let mut per_record = Vec::new();
    for record in 0..records {
        buffers.set_time(0, i32::try_from(record * 60)?)?;
        let started = std::time::Instant::now();
        buffers.write_gridded_record(&path, records, record)?;
        let seconds = started.elapsed().as_secs_f64();
        per_record.push(seconds);
        println!("record {record}: {seconds:.3} s");
    }
    let bytes = std::fs::metadata(&path)?.len();
    let total: f64 = per_record.iter().sum();
    println!(
        "wrote {} bytes in {total:.3} s; {:.1} MB/s; {:.1} MB/record ({} vars); \
         extrapolated 221 vars: {:.1} s/record, {:.2} GB/record",
        bytes,
        bytes as f64 / total / 1.0e6,
        bytes as f64 / records as f64 / 1.0e6,
        names.len(),
        221.0 / names.len() as f64 * total / records as f64,
        221.0 / names.len() as f64 * bytes as f64 / records as f64 / 1.0e9,
    );
    Ok(())
}
