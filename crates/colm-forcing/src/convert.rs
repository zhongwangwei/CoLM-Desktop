//! 把一份强迫场文件转成 CoLM 认的约定。
//!
//! **只转认不出来的数据。** PLUMBER2 继续直读（`lib.rs` 开头那段说明），
//! 转它只会多一份 50 MB 拷贝和一次误差机会，而黄金回归正靠它。
//!
//! **产物与源文件分开存放，原始数据永不改动**（前处理页立的约束）。

use std::path::Path;

use anyhow::{Context, Result};

/// 原样复制一份强迫场文件。
///
/// **这是转换管道的地基，也是它的第一条判据。** 恒等转换必须逐位复现 ——
/// 若这一步就丢精度，后面所有换算的正确性都无从谈起。
///
/// 实现上是「读出来再写进去」而不是 `std::fs::copy`：`fs::copy` 复现的是
/// 字节，证明不了「我们的读写路径不丢精度」，而后者才是要验的东西。
pub fn identity(src: &Path, dst: &Path) -> Result<()> {
    let fin = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let mut fout =
        netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;

    for d in fin.dimensions() {
        fout.add_dimension(&d.name(), d.len())
            .with_context(|| format!("cannot add dimension {}", d.name()))?;
    }

    for v in fin.variables() {
        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let values: Vec<f64> = v
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {}", v.name()))?;
        let mut out = fout
            .add_variable::<f64>(&v.name(), &dim_refs)
            .with_context(|| format!("cannot add variable {}", v.name()))?;
        for a in v.attributes() {
            if let Ok(netcdf::AttributeValue::Str(s)) = a.value() {
                out.put_attribute(a.name(), s.as_str())?;
            }
        }
        out.put_values(&values, netcdf::Extents::All)
            .with_context(|| format!("cannot write {}", v.name()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod convert_tests;
