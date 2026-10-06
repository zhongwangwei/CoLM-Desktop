//! netCDF-4 变量的并行压缩写入。
//!
//! netCDF-4/HDF5 的 deflate 在写盘时单线程逐块压缩。高分辨率输出时这一段成了瓶颈：真实全球场
//! 实测串行 deflate 1 只有约 86 MB/s，落盘本身却有 1.7 GB/s（第 602 轮）。这里换一种写法，
//! 文件仍是同一个标准 netCDF-4 文件：
//!
//! 1. 调用方照常用 netcdf crate 定义变量，块形状取 [`chunk_shape`]，压缩级别照旧，然后关闭文件；
//! 2. [`pack_f64`] 按块切开数据，用 rayon 并行压缩（与 HDF5 的 deflate 过滤器同为 zlib 的 `compress2`），
//!    可以几个变量同时压；[`ChunkWriter::write`] 再用 HDF5 的 `H5Dwrite_chunk` 把压好的字节直接写进去。
//!
//! 读者（Fortran、GUI、xarray、ncview）看到的是普通的 deflate 变量，解压后的值与 `put_values` 写出的
//! 逐位相同；只有压缩字节不一定与 libz 串行压出来的一样，块形状也不再是 netCDF 的默认值。
//!
//! HDF5 不是线程安全构建：所有 HDF5 调用都在 netcdf crate 用的那把全局锁（`hdf5_sys::LOCK`）下做，
//! 与其它线程（例如强迫预读）的 netCDF 调用串行。压缩不碰 HDF5，在锁外并行。

use std::path::Path;

use anyhow::{ensure, Context, Result};
use rayon::prelude::*;

#[cfg(target_endian = "big")]
compile_error!("colm-h5chunk writes little-endian f64 chunks (H5T_IEEE_F64LE)");

/// 一块的目标大小。netCDF 默认的块也是这个量级；块太大，只读一小片区域的读者也得解压整块。
pub const TARGET_CHUNK_BYTES: usize = 4 << 20;

/// 形状为 `shape`（第一维是时间）的 f64 变量的块形状：时间取 1，其余维从外往里收缩，直到一块
/// 不超过 [`TARGET_CHUNK_BYTES`]。最内层维尽量保持完整，块在内存里因此是连续的几段。
pub fn chunk_shape(shape: &[usize]) -> Vec<usize> {
    chunk_shape_for(shape, TARGET_CHUNK_BYTES)
}

fn chunk_shape_for(shape: &[usize], target_bytes: usize) -> Vec<usize> {
    let mut chunk: Vec<usize> = shape.iter().map(|&n| n.max(1)).collect();
    if let Some(time) = chunk.first_mut() {
        *time = 1;
    }
    let target = (target_bytes / std::mem::size_of::<f64>()).max(1);
    for axis in 1..chunk.len() {
        let inner: usize = chunk[axis + 1..].iter().product();
        if chunk[axis] * inner <= target {
            break;
        }
        chunk[axis] = (target / inner).max(1);
    }
    chunk
}

/// 一个已关闭的 netCDF-4 文件，以读写方式重新打开，用来直写块。
pub struct ChunkWriter {
    file: ffi::Handle,
}

impl ChunkWriter {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            file: ffi::open_rw(path)
                .with_context(|| format!("cannot reopen {} for chunk writes", path.display()))?,
        })
    }

    /// 把 `data`（按 `shape` 行主序）写进已定义的变量 `name`：[`pack_f64`] 加 [`Self::write`]。
    pub fn write_f64(
        &mut self,
        name: &str,
        shape: &[usize],
        chunk: &[usize],
        data: &[f64],
        level: u8,
    ) -> Result<()> {
        self.write(&pack_f64(name, shape, chunk, data, level)?)
    }

    /// 把压好的变量写进文件：把无限维扩到 `shape[0]`，逐块直写。
    pub fn write(&mut self, packed: &PackedVariable) -> Result<()> {
        let name = &packed.name;
        let dataset = ffi::open_dataset(&self.file, name)?;
        ffi::set_extent(&dataset, &packed.shape).with_context(|| format!("cannot size {name}"))?;
        for (origin, bytes) in &packed.chunks {
            ffi::write_chunk(&dataset, origin, bytes)
                .with_context(|| format!("cannot write a chunk of {name} at {origin:?}"))?;
        }
        Ok(())
    }

    pub fn close(self) -> Result<()> {
        ffi::close_file(self.file)
    }
}

/// 压好、等着写盘的一个变量。
#[derive(Debug)]
pub struct PackedVariable {
    name: String,
    shape: Vec<usize>,
    /// 每块的起点与压好的字节。
    chunks: Vec<(Vec<usize>, Vec<u8>)>,
}

/// 按块切开 `data`（按 `shape` 行主序）并用 rayon 并行压缩。不碰 HDF5，可以在任何线程、对多个变量
/// 同时调用；变量必须按 `chunk` 分块、`level` 级 deflate、不开 shuffle，第一维是无限维。
pub fn pack_f64(
    name: &str,
    shape: &[usize],
    chunk: &[usize],
    data: &[f64],
    level: u8,
) -> Result<PackedVariable> {
    ensure!(
        shape.len() == chunk.len() && !shape.is_empty(),
        "{name}: chunk rank {} does not match shape rank {}",
        chunk.len(),
        shape.len()
    );
    ensure!(
        chunk.iter().all(|&c| c > 0),
        "{name}: chunk sizes must be positive"
    );
    ensure!(
        data.len() == shape.iter().product::<usize>(),
        "{name}: {} values for shape {shape:?}",
        data.len()
    );
    ensure!((1..=9).contains(&level), "{name}: deflate level {level}");
    let grid: Vec<usize> = shape
        .iter()
        .zip(chunk)
        .map(|(&n, &c)| n.div_ceil(c))
        .collect();
    let count: usize = grid.iter().product();
    let chunks = (0..count)
        .into_par_iter()
        .map(|index| {
            let origin = chunk_origin(index, &grid, chunk);
            let bytes = chunk_bytes(data, shape, chunk, &origin);
            ffi::deflate(&bytes, level).map(|packed| (origin, packed))
        })
        .collect::<Result<Vec<_>>>()
        .with_context(|| format!("cannot compress {name}"))?;
    Ok(PackedVariable {
        name: name.to_owned(),
        shape: shape.to_vec(),
        chunks,
    })
}

/// 第 `index` 块（行主序编号）的起点。
fn chunk_origin(mut index: usize, grid: &[usize], chunk: &[usize]) -> Vec<usize> {
    let mut origin = vec![0; grid.len()];
    for axis in (0..grid.len()).rev() {
        origin[axis] = (index % grid[axis]) * chunk[axis];
        index /= grid[axis];
    }
    origin
}

/// 一整块的字节（小端 f64）。越出数据范围的边缘部分照 HDF5 的要求补满整块，补的值读者看不到。
fn chunk_bytes(data: &[f64], shape: &[usize], chunk: &[usize], origin: &[usize]) -> Vec<u8> {
    let rank = shape.len();
    let inner = rank - 1;
    let mut values = vec![0.0f64; chunk.iter().product()];
    // 行主序的步长。
    let mut stride = vec![1usize; rank];
    let mut chunk_stride = vec![1usize; rank];
    for axis in (0..inner).rev() {
        stride[axis] = stride[axis + 1] * shape[axis + 1];
        chunk_stride[axis] = chunk_stride[axis + 1] * chunk[axis + 1];
    }
    let run = chunk[inner].min(shape[inner] - origin[inner]);
    // 遍历块内除最内层外的各维（只到数据范围内），每次拷最内层连续的一段。
    let extent: Vec<usize> = (0..inner)
        .map(|axis| chunk[axis].min(shape[axis] - origin[axis]))
        .collect();
    let mut at = vec![0usize; inner];
    loop {
        let mut source = origin[inner];
        let mut target = 0;
        for axis in 0..inner {
            source += (origin[axis] + at[axis]) * stride[axis];
            target += at[axis] * chunk_stride[axis];
        }
        values[target..target + run].copy_from_slice(&data[source..source + run]);
        // 进位。
        let mut axis = inner;
        loop {
            if axis == 0 {
                return values.iter().flat_map(|v| v.to_le_bytes()).collect();
            }
            axis -= 1;
            at[axis] += 1;
            if at[axis] < extent[axis] {
                break;
            }
            at[axis] = 0;
        }
    }
}

/// HDF5 与 zlib 的 FFI。本 crate 只有这里有 unsafe。
#[allow(unsafe_code)]
mod ffi {
    use std::ffi::CString;
    use std::path::Path;

    use anyhow::{bail, ensure, Context, Result};
    use hdf5_sys::h5::hsize_t;
    use hdf5_sys::h5d::{H5Dclose, H5Dopen2, H5Dset_extent, H5Dwrite_chunk};
    use hdf5_sys::h5f::{H5Fclose, H5Fopen, H5F_ACC_RDWR};
    use hdf5_sys::h5i::hid_t;
    use hdf5_sys::h5p::H5P_DEFAULT;

    /// 一个打开的 HDF5 文件或数据集；`Drop` 时在锁下关闭。
    pub struct Handle {
        id: hid_t,
        close: unsafe extern "C" fn(hid_t) -> hdf5_sys::h5::herr_t,
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            if self.id >= 0 {
                let _lock = hdf5_sys::LOCK.lock();
                // SAFETY: `id` 是这个 Handle 独占的、仍然打开的 HDF5 标识符，关一次后置为 -1。
                unsafe { (self.close)(self.id) };
                self.id = -1;
            }
        }
    }

    pub fn open_rw(path: &Path) -> Result<Handle> {
        let name = CString::new(path.to_str().context("path is not UTF-8")?)?;
        let _lock = hdf5_sys::LOCK.lock();
        // SAFETY: `name` 是以 NUL 结尾的 C 字符串，在调用期间有效；HDF5 调用在全局锁下串行。
        let id = unsafe { H5Fopen(name.as_ptr(), H5F_ACC_RDWR, H5P_DEFAULT) };
        ensure!(id >= 0, "H5Fopen failed");
        Ok(Handle {
            id,
            close: H5Fclose,
        })
    }

    pub fn close_file(mut file: Handle) -> Result<()> {
        let _lock = hdf5_sys::LOCK.lock();
        // SAFETY: 同 `Drop`；关闭后置 -1，`Drop` 不再关第二次。
        let status = unsafe { H5Fclose(file.id) };
        file.id = -1;
        ensure!(status >= 0, "H5Fclose failed");
        Ok(())
    }

    pub fn open_dataset(file: &Handle, name: &str) -> Result<Handle> {
        let cname = CString::new(name)?;
        let _lock = hdf5_sys::LOCK.lock();
        // SAFETY: `file.id` 是打开的文件；`cname` 在调用期间有效。
        let id = unsafe { H5Dopen2(file.id, cname.as_ptr(), H5P_DEFAULT) };
        ensure!(id >= 0, "variable {name} is not a dataset in this file");
        Ok(Handle {
            id,
            close: H5Dclose,
        })
    }

    fn dims(values: &[usize]) -> Result<Vec<hsize_t>> {
        values
            .iter()
            .map(|&v| hsize_t::try_from(v).context("dimension does not fit hsize_t"))
            .collect()
    }

    pub fn set_extent(dataset: &Handle, shape: &[usize]) -> Result<()> {
        let size = dims(shape)?;
        let _lock = hdf5_sys::LOCK.lock();
        // SAFETY: `size` 的长度等于数据集的秩（调用方按变量的维数传入），在调用期间有效。
        let status = unsafe { H5Dset_extent(dataset.id, size.as_ptr()) };
        ensure!(status >= 0, "H5Dset_extent failed");
        Ok(())
    }

    pub fn write_chunk(dataset: &Handle, origin: &[usize], bytes: &[u8]) -> Result<()> {
        let offset = dims(origin)?;
        let _lock = hdf5_sys::LOCK.lock();
        // SAFETY: `offset` 的长度等于数据集的秩、指向块的起点；`bytes` 是一整块按数据集的过滤器
        // （仅 deflate）压好的数据，过滤器掩码 0 表示全部过滤器都已应用。两者在调用期间有效。
        let status = unsafe {
            H5Dwrite_chunk(
                dataset.id,
                H5P_DEFAULT,
                0,
                offset.as_ptr(),
                bytes.len(),
                bytes.as_ptr().cast(),
            )
        };
        ensure!(status >= 0, "H5Dwrite_chunk failed");
        Ok(())
    }

    /// zlib 格式的 deflate（`compress2`），与 HDF5 的 deflate 过滤器相同。不碰 HDF5，可以并行调用。
    pub fn deflate(source: &[u8], level: u8) -> Result<Vec<u8>> {
        let source_len =
            libz_sys::uLong::try_from(source.len()).context("chunk too large for zlib")?;
        // SAFETY: `compressBound` 只做算术。
        let bound = unsafe { libz_sys::compressBound(source_len) };
        let mut packed = vec![0u8; usize::try_from(bound)?];
        let mut packed_len = bound;
        // SAFETY: `packed` 有 `bound` 个字节可写，`source` 有 `source_len` 个字节可读；zlib 不保留指针。
        let status = unsafe {
            libz_sys::compress2(
                packed.as_mut_ptr(),
                &mut packed_len,
                source.as_ptr(),
                source_len,
                i32::from(level),
            )
        };
        if status != libz_sys::Z_OK {
            bail!("zlib compress2 returned {status}");
        }
        packed.truncate(usize::try_from(packed_len)?);
        Ok(packed)
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
