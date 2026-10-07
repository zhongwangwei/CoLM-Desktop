//! 原始数据的并行读取。
//!
//! netcdf crate 读压缩变量时，HDF5 在全局锁里逐块解压：多线程读原始栅格实际是一个一个排队，
//! mksrfdata 在大区域上大半时间只用一个核（第 603 轮实测）。这里反过来做：
//!
//! 1. 锁里只做 I/O：查出区域涉及的块的存储大小（`H5Dget_chunk_storage_size`），读出压缩字节（`H5Dread_chunk1`）；
//! 2. 锁外用 rayon 并行解压（inflate，再按需 unshuffle），转成目标类型，只截取与区域相交的那部分；
//! 3. 主线程把各块的相交部分拼成区域。
//!
//! 只在结果与 netCDF `get_values` 逐位相同时走这条路：分块存储、过滤器只有 shuffle/deflate、小端、
//! 所有块都已分配，类型转换只接受不会越界且精确的组合（见 [`Element`]）。其余情况一律回落到
//! netCDF 的 `get_values`，结果照旧。

use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;

/// 一次在锁里读多少块的压缩字节：限住同时在内存里的压缩数据。
const BATCH_CHUNKS: usize = 256;

/// 文件里的数值类型（小端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl FileType {
    fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

/// 能从文件类型精确转换过来的目标类型。`decode` 返回 `None` 表示这个组合不走快路径（可能越界或
/// 不精确，交给 netCDF 按它自己的规则转换并报错）。
pub trait Element: Copy + Send + Sync + netcdf::NcTypeDescriptor + 'static {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self>;
}

fn le<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes[..N].try_into().expect("element width")
}

impl Element for f64 {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self> {
        Some(match file {
            FileType::F64 => |b| f64::from_le_bytes(le(b)),
            FileType::F32 => |b| f64::from(f32::from_le_bytes(le(b))),
            FileType::I8 => |b| f64::from(i8::from_le_bytes(le(b))),
            FileType::U8 => |b| f64::from(b[0]),
            FileType::I16 => |b| f64::from(i16::from_le_bytes(le(b))),
            FileType::U16 => |b| f64::from(u16::from_le_bytes(le(b))),
            FileType::I32 => |b| f64::from(i32::from_le_bytes(le(b))),
            FileType::U32 => |b| f64::from(u32::from_le_bytes(le(b))),
            FileType::I64 | FileType::U64 => return None,
        })
    }
}

impl Element for f32 {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self> {
        Some(match file {
            FileType::F32 => |b| f32::from_le_bytes(le(b)),
            FileType::I8 => |b| f32::from(i8::from_le_bytes(le(b))),
            FileType::U8 => |b| f32::from(b[0]),
            FileType::I16 => |b| f32::from(i16::from_le_bytes(le(b))),
            FileType::U16 => |b| f32::from(u16::from_le_bytes(le(b))),
            _ => return None,
        })
    }
}

impl Element for i64 {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self> {
        Some(match file {
            FileType::I64 => |b| i64::from_le_bytes(le(b)),
            FileType::I8 => |b| i64::from(i8::from_le_bytes(le(b))),
            FileType::U8 => |b| i64::from(b[0]),
            FileType::I16 => |b| i64::from(i16::from_le_bytes(le(b))),
            FileType::U16 => |b| i64::from(u16::from_le_bytes(le(b))),
            FileType::I32 => |b| i64::from(i32::from_le_bytes(le(b))),
            FileType::U32 => |b| i64::from(u32::from_le_bytes(le(b))),
            _ => return None,
        })
    }
}

impl Element for i32 {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self> {
        Some(match file {
            FileType::I32 => |b| i32::from_le_bytes(le(b)),
            FileType::I8 => |b| i32::from(i8::from_le_bytes(le(b))),
            FileType::U8 => |b| i32::from(b[0]),
            FileType::I16 => |b| i32::from(i16::from_le_bytes(le(b))),
            FileType::U16 => |b| i32::from(u16::from_le_bytes(le(b))),
            _ => return None,
        })
    }
}

impl Element for i16 {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self> {
        Some(match file {
            FileType::I16 => |b| i16::from_le_bytes(le(b)),
            FileType::I8 => |b| i16::from(i8::from_le_bytes(le(b))),
            FileType::U8 => |b| i16::from(b[0]),
            _ => return None,
        })
    }
}

impl Element for i8 {
    fn decode(file: FileType) -> Option<fn(&[u8]) -> Self> {
        Some(match file {
            FileType::I8 => |b| i8::from_le_bytes(le(b)),
            _ => return None,
        })
    }
}

/// 读变量 `variable` 的一个超矩形（`start`/`count` 按文件维度顺序），结果行主序。能走快路径就并行
/// 解压，否则照常用 netCDF `get_values`。两条路结果逐位相同。
pub fn read_region<T: Element>(
    path: &Path,
    variable: &str,
    start: &[usize],
    count: &[usize],
) -> Result<Vec<T>> {
    if let Some(values) = read_region_fast(path, variable, start, count)? {
        return Ok(values);
    }
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", path.display()))?;
    let extents: Vec<netcdf::Extent> = start
        .iter()
        .zip(count)
        .map(|(&start, &count)| netcdf::Extent::SliceCount {
            start,
            count,
            stride: 1,
        })
        .collect();
    Ok(source.get_values::<T, _>(extents)?)
}

/// 快路径：条件不满足时返回 `Ok(None)`（调用方回落）；块解压出错是真错误。
pub fn read_region_fast<T: Element>(
    path: &Path,
    variable: &str,
    start: &[usize],
    count: &[usize],
) -> Result<Option<Vec<T>>> {
    if start.len() != count.len() || start.is_empty() {
        return Ok(None);
    }
    let Some(dataset) = ffi::Dataset::open(path, variable) else {
        return Ok(None);
    };
    let Some(layout) = dataset.layout() else {
        return Ok(None);
    };
    let rank = start.len();
    if layout.shape.len() != rank
        || start
            .iter()
            .zip(count)
            .zip(&layout.shape)
            .any(|((&s, &c), &n)| s.checked_add(c).is_none_or(|end| end > n))
    {
        return Ok(None);
    }
    let Some(decode) = T::decode(layout.file_type) else {
        return Ok(None);
    };
    let total: usize = count.iter().product();
    let mut output: Vec<T> = Vec::with_capacity(total);
    if total == 0 {
        return Ok(Some(output));
    }
    // 区域涉及的块（行主序）。
    let first: Vec<usize> = start
        .iter()
        .zip(&layout.chunk)
        .map(|(&s, &c)| s / c)
        .collect();
    let last: Vec<usize> = start
        .iter()
        .zip(count)
        .zip(&layout.chunk)
        .map(|((&s, &k), &c)| (s + k - 1) / c)
        .collect();
    let mut chunks = Vec::new();
    let mut at = first.clone();
    loop {
        chunks.push(
            at.iter()
                .zip(&layout.chunk)
                .map(|(&i, &c)| i * c)
                .collect::<Vec<_>>(),
        );
        let mut axis = rank;
        loop {
            if axis == 0 {
                break;
            }
            axis -= 1;
            at[axis] += 1;
            if at[axis] <= last[axis] {
                break;
            }
            at[axis] = first[axis];
        }
        if at == first {
            break;
        }
    }
    let element = layout.file_type.size();
    let chunk_elements: usize = layout.chunk.iter().product();
    let mut pieces: Vec<(Vec<usize>, Vec<usize>, Vec<T>)> = Vec::with_capacity(chunks.len());
    for batch in chunks.chunks(BATCH_CHUNKS) {
        let Some(raw) = dataset.read_chunks(batch) else {
            return Ok(None);
        };
        let decoded = batch
            .par_iter()
            .zip(raw.into_par_iter())
            .map(|(origin, (mask, bytes))| {
                let bytes = layout.unfilter(bytes, mask, chunk_elements * element)?;
                Ok(intersect(
                    &bytes,
                    element,
                    decode,
                    origin,
                    &layout.chunk,
                    start,
                    count,
                ))
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("cannot decode {variable} in {}", path.display()))?;
        pieces.extend(decoded);
    }
    // 拼成区域：先整片填一个占位值，再按各块的相交部分覆盖。块覆盖整个区域（都已分配），所以每个
    // 位置都会被覆盖。
    let seed = pieces
        .iter()
        .find_map(|(_, _, values)| values.first().copied())
        .context("region has no values")?;
    output.resize(total, seed);
    let mut stride = vec![1usize; rank];
    for axis in (0..rank - 1).rev() {
        stride[axis] = stride[axis + 1] * count[axis + 1];
    }
    for (lo, extent, values) in &pieces {
        let run = extent[rank - 1];
        let rows: usize = extent[..rank - 1].iter().product();
        let mut index = vec![0usize; rank - 1];
        for row in 0..rows {
            let mut target = lo[rank - 1] - start[rank - 1];
            for axis in 0..rank - 1 {
                target += (lo[axis] - start[axis] + index[axis]) * stride[axis];
            }
            output[target..target + run].copy_from_slice(&values[row * run..(row + 1) * run]);
            for axis in (0..rank - 1).rev() {
                index[axis] += 1;
                if index[axis] < extent[axis] {
                    break;
                }
                index[axis] = 0;
            }
        }
    }
    Ok(Some(output))
}

/// 一块解压后的字节里与区域相交的部分：`(相交区起点, 相交区长度, 值)`，值按相交区行主序。
fn intersect<T: Copy>(
    bytes: &[u8],
    element: usize,
    decode: fn(&[u8]) -> T,
    origin: &[usize],
    chunk: &[usize],
    start: &[usize],
    count: &[usize],
) -> (Vec<usize>, Vec<usize>, Vec<T>) {
    let rank = origin.len();
    let lo: Vec<usize> = (0..rank).map(|a| origin[a].max(start[a])).collect();
    let hi: Vec<usize> = (0..rank)
        .map(|a| (origin[a] + chunk[a]).min(start[a] + count[a]))
        .collect();
    let extent: Vec<usize> = (0..rank).map(|a| hi[a] - lo[a]).collect();
    let mut chunk_stride = vec![1usize; rank];
    for axis in (0..rank - 1).rev() {
        chunk_stride[axis] = chunk_stride[axis + 1] * chunk[axis + 1];
    }
    let run = extent[rank - 1];
    let rows: usize = extent[..rank - 1].iter().product();
    let mut values = Vec::with_capacity(rows * run);
    let mut index = vec![0usize; rank - 1];
    for _ in 0..rows {
        let mut source = lo[rank - 1] - origin[rank - 1];
        for axis in 0..rank - 1 {
            source += (lo[axis] - origin[axis] + index[axis]) * chunk_stride[axis];
        }
        values.extend(
            bytes[source * element..(source + run) * element]
                .chunks_exact(element)
                .map(decode),
        );
        for axis in (0..rank - 1).rev() {
            index[axis] += 1;
            if index[axis] < extent[axis] {
                break;
            }
            index[axis] = 0;
        }
    }
    (lo, extent, values)
}

/// 一个数据集的存储布局与过滤器。
pub(crate) struct Layout {
    pub(crate) shape: Vec<usize>,
    pub(crate) chunk: Vec<usize>,
    pub(crate) file_type: FileType,
    /// 过滤器在管线里的位置（掩码位）。管线顺序必须是先 shuffle 后 deflate。
    pub(crate) shuffle: Option<u32>,
    pub(crate) deflate: Option<u32>,
}

impl Layout {
    /// 写入时依次 shuffle、deflate；读取反过来。`mask` 的第 i 位表示管线第 i 个过滤器在这一块上跳过了。
    fn unfilter(&self, bytes: Vec<u8>, mask: u32, raw_len: usize) -> Result<Vec<u8>> {
        let applied = |position: Option<u32>| position.is_some_and(|i| mask & (1 << i) == 0);
        let bytes = if applied(self.deflate) {
            ffi::inflate(&bytes, raw_len)?
        } else {
            bytes
        };
        anyhow::ensure!(
            bytes.len() == raw_len,
            "chunk decodes to {} bytes, expected {raw_len}",
            bytes.len()
        );
        if !applied(self.shuffle) {
            return Ok(bytes);
        }
        let width = self.file_type.size();
        if width == 1 {
            return Ok(bytes);
        }
        let n = raw_len / width;
        let mut out = vec![0u8; raw_len];
        for b in 0..width {
            let plane = &bytes[b * n..(b + 1) * n];
            for (i, &byte) in plane.iter().enumerate() {
                out[i * width + b] = byte;
            }
        }
        Ok(out)
    }
}

/// 读取侧的 HDF5 与 zlib FFI。本模块只有这里有 unsafe。
#[allow(unsafe_code)]
mod ffi {
    use std::ffi::CString;
    use std::path::Path;
    use std::sync::Once;

    use anyhow::{bail, Result};
    use hdf5_sys::h5::hsize_t;
    use hdf5_sys::h5d::{
        H5D_layout_t, H5Dclose, H5Dget_create_plist, H5Dget_space, H5Dget_type, H5Dopen2,
        H5Dread_chunk1,
    };
    use hdf5_sys::h5e::{H5Eset_auto2, H5E_DEFAULT};
    use hdf5_sys::h5f::{H5Fclose, H5Fopen, H5F_ACC_RDONLY};
    use hdf5_sys::h5p::{
        H5Pclose, H5Pget_chunk, H5Pget_filter2, H5Pget_layout, H5Pget_nfilters, H5P_DEFAULT,
    };
    use hdf5_sys::h5s::{H5Sclose, H5Sget_simple_extent_dims, H5Sget_simple_extent_ndims};
    use hdf5_sys::h5t::{
        H5T_class_t, H5T_order_t, H5T_sign_t, H5Tclose, H5Tget_class, H5Tget_order, H5Tget_sign,
        H5Tget_size,
    };
    use hdf5_sys::h5z::{H5Z_FILTER_DEFLATE, H5Z_FILTER_SHUFFLE};

    use super::{FileType, Layout};
    use crate::ffi::Handle;

    unsafe extern "C" {
        /// HDF5 1.10.2 起的公开 API，hdf5-sys 没有声明。按块索引查找（B 树），不像
        /// `H5Dget_chunk_info_by_coord` 那样遍历全部块：15″ LAI 一个变量约 200 万块，后者每块要几毫秒。
        fn H5Dget_chunk_storage_size(
            dset_id: hdf5_sys::h5i::hid_t,
            offset: *const hsize_t,
            chunk_bytes: *mut hsize_t,
        ) -> hdf5_sys::h5::herr_t;
    }

    /// HDF5 的错误栈打印：快路径的失败（例如 netCDF-3 文件打不开）是预期的回落，不该刷到 stderr。
    fn quiet() {
        static QUIET: Once = Once::new();
        QUIET.call_once(|| {
            // SAFETY: 关掉默认错误栈的自动打印；在全局锁下调用。
            unsafe { H5Eset_auto2(H5E_DEFAULT, None, std::ptr::null_mut()) };
        });
    }

    pub struct Dataset {
        // 先关数据集再关文件：字段按声明顺序析构。
        dataset: Handle,
        _file: Handle,
    }

    impl Dataset {
        pub fn open(path: &Path, variable: &str) -> Option<Self> {
            let name = CString::new(path.to_str()?).ok()?;
            let cvar = CString::new(variable).ok()?;
            let _lock = hdf5_sys::LOCK.lock();
            quiet();
            // SAFETY: 两个 C 字符串在调用期间有效；只读打开，HDF5 调用在全局锁下串行。
            let file = unsafe { H5Fopen(name.as_ptr(), H5F_ACC_RDONLY, H5P_DEFAULT) };
            if file < 0 {
                return None;
            }
            let file = Handle {
                id: file,
                close: H5Fclose,
            };
            // SAFETY: `file.id` 是打开的文件。
            let dataset = unsafe { H5Dopen2(file.id, cvar.as_ptr(), H5P_DEFAULT) };
            if dataset < 0 {
                return None;
            }
            Some(Self {
                dataset: Handle {
                    id: dataset,
                    close: H5Dclose,
                },
                _file: file,
            })
        }

        pub fn layout(&self) -> Option<Layout> {
            let _lock = hdf5_sys::LOCK.lock();
            let id = self.dataset.id;
            // SAFETY: 下面每个调用的参数都是本数据集或刚取得、随后关闭的属性表/数据类型/数据空间。
            unsafe {
                let space = H5Dget_space(id);
                if space < 0 {
                    return None;
                }
                let rank = H5Sget_simple_extent_ndims(space);
                let mut shape = vec![0 as hsize_t; rank.max(0) as usize];
                let ok = rank > 0
                    && H5Sget_simple_extent_dims(space, shape.as_mut_ptr(), std::ptr::null_mut())
                        == rank;
                H5Sclose(space);
                if !ok {
                    return None;
                }
                let plist = H5Dget_create_plist(id);
                if plist < 0 {
                    return None;
                }
                let layout = (|| {
                    if H5Pget_layout(plist) != H5D_layout_t::H5D_CHUNKED {
                        return None;
                    }
                    let mut chunk = vec![0 as hsize_t; rank as usize];
                    if H5Pget_chunk(plist, rank, chunk.as_mut_ptr()) != rank {
                        return None;
                    }
                    let mut shuffle = None;
                    let mut deflate = None;
                    let filters = H5Pget_nfilters(plist);
                    for index in 0..filters.max(0) as u32 {
                        let mut flags = 0;
                        let mut nelmts = 0;
                        let mut config = 0;
                        let filter = H5Pget_filter2(
                            plist,
                            index,
                            &mut flags,
                            &mut nelmts,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                            &mut config,
                        );
                        match filter {
                            H5Z_FILTER_SHUFFLE if deflate.is_none() => shuffle = Some(index),
                            H5Z_FILTER_DEFLATE => deflate = Some(index),
                            _ => return None,
                        }
                    }
                    let datatype = H5Dget_type(id);
                    if datatype < 0 {
                        return None;
                    }
                    let class = H5Tget_class(datatype);
                    let size = H5Tget_size(datatype);
                    let order = H5Tget_order(datatype);
                    let sign = H5Tget_sign(datatype);
                    H5Tclose(datatype);
                    if order != H5T_order_t::H5T_ORDER_LE {
                        return None;
                    }
                    let signed = sign == H5T_sign_t::H5T_SGN_2;
                    let file_type = match (class, size, signed) {
                        (H5T_class_t::H5T_FLOAT, 4, _) => FileType::F32,
                        (H5T_class_t::H5T_FLOAT, 8, _) => FileType::F64,
                        (H5T_class_t::H5T_INTEGER, 1, true) => FileType::I8,
                        (H5T_class_t::H5T_INTEGER, 1, false) => FileType::U8,
                        (H5T_class_t::H5T_INTEGER, 2, true) => FileType::I16,
                        (H5T_class_t::H5T_INTEGER, 2, false) => FileType::U16,
                        (H5T_class_t::H5T_INTEGER, 4, true) => FileType::I32,
                        (H5T_class_t::H5T_INTEGER, 4, false) => FileType::U32,
                        (H5T_class_t::H5T_INTEGER, 8, true) => FileType::I64,
                        (H5T_class_t::H5T_INTEGER, 8, false) => FileType::U64,
                        _ => return None,
                    };
                    Some(Layout {
                        shape: shape.iter().map(|&n| n as usize).collect(),
                        chunk: chunk.iter().map(|&c| c as usize).collect(),
                        file_type,
                        shuffle,
                        deflate,
                    })
                })();
                H5Pclose(plist);
                layout.filter(|layout| layout.chunk.iter().all(|&c| c > 0))
            }
        }

        /// 一批块的 `(过滤器掩码, 压缩字节)`；有块没分配（读出来该是填充值）时返回 `None`，交给 netCDF。
        pub fn read_chunks(&self, origins: &[Vec<usize>]) -> Option<Vec<(u32, Vec<u8>)>> {
            let _lock = hdf5_sys::LOCK.lock();
            let mut out = Vec::with_capacity(origins.len());
            for origin in origins {
                let offset: Vec<hsize_t> = origin.iter().map(|&o| o as hsize_t).collect();
                let mut size: hsize_t = 0;
                // SAFETY: `offset` 的长度等于数据集的秩、是块的起点；`size` 指向本地变量。
                let status = unsafe {
                    H5Dget_chunk_storage_size(self.dataset.id, offset.as_ptr(), &mut size)
                };
                // 未分配的块存储大小为 0：读出来该是填充值，交给 netCDF。
                if status < 0 || size == 0 {
                    return None;
                }
                let mut bytes = vec![0u8; usize::try_from(size).ok()?];
                let mut filters = 0u32;
                // SAFETY: `bytes` 正好是这一块的存储大小；`offset` 同上。
                let status = unsafe {
                    H5Dread_chunk1(
                        self.dataset.id,
                        H5P_DEFAULT,
                        offset.as_ptr(),
                        &mut filters,
                        bytes.as_mut_ptr().cast(),
                    )
                };
                if status < 0 {
                    return None;
                }
                out.push((filters, bytes));
            }
            Some(out)
        }
    }

    /// zlib 格式的 inflate（HDF5 deflate 过滤器的逆）。不碰 HDF5，可以并行调用。
    pub fn inflate(source: &[u8], raw_len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; raw_len];
        let mut out_len = libz_sys::uLong::try_from(raw_len)?;
        let source_len = libz_sys::uLong::try_from(source.len())?;
        // SAFETY: `out` 有 `raw_len` 个字节可写，`source` 有 `source_len` 个字节可读；zlib 不保留指针。
        let status = unsafe {
            libz_sys::uncompress(out.as_mut_ptr(), &mut out_len, source.as_ptr(), source_len)
        };
        if status != libz_sys::Z_OK {
            bail!("zlib uncompress returned {status}");
        }
        out.truncate(usize::try_from(out_len)?);
        Ok(out)
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod read_tests;
