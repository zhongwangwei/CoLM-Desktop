//! NetCDF 的 `NC_CHAR` 元素类型。
//!
//! 区域单元流域网络要原样拷贝 `dam_DamName(dam_ndams, dam_namelen)` 这类字符数组，Fortran 写的是
//! `NC_CHAR`。netcdf crate 只内置数值类型，`put_string` 写的是 netCDF-4 的 `NC_STRING`，类型不同；
//! 按 crate 文档自定义一个透明包装 `i8` 的类型即可按 `NC_CHAR` 读写。

/// 一个 `NC_CHAR` 字节（C 的 `char`）。
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NcChar(pub i8);

// SAFETY: `NcTypeDescriptor` 要求类型的内存布局与它报告的 netCDF 类型一致。`NcChar` 是
// `#[repr(transparent)]` 的 `i8`，与 `NC_CHAR` 的 C `char` 同为 1 字节、无填充、任意位型都合法。
#[allow(unsafe_code)]
unsafe impl netcdf::types::NcTypeDescriptor for NcChar {
    fn type_descriptor() -> netcdf::types::NcVariableType {
        netcdf::types::NcVariableType::Char
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
