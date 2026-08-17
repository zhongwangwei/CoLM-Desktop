//! 读一个 PLUMBER2 站点文件，补齐 12 个字段，写出增广站点文件。
//!
//! 做法是「拷贝后追加」而不是重建：站点文件里那 39 个变量连同它们的属性、
//! 维度、压缩设置都必须原样保留，重建一份等于把上游数据重新表述一遍，
//! 而任何一处表述差异都会变成一个没人发现的数值差异。
//!
//! 每个补进去的变量都带一个 `source` 属性，写明它是量出来的还是假设的。

use std::path::Path;

use anyhow::{Context, Result};

use crate::albedo::albedo;
use crate::derive::{derive, fine_earth_fractions, SoilColumn};
use crate::raster::{point_f64, point_i32};
use crate::texture::{classify, BVIC_USDA, CLASS_NAMES};

/// CoLM 无条件读取而 PLUMBER2 站点文件不提供的 12 个字段。
pub const REQUIRED_FIELDS: [&str; 12] = [
    "elevation",
    "elvstd",
    "lakedepth",
    "sloperatio",
    "soil_s_v_alb",
    "soil_d_v_alb",
    "soil_s_n_alb",
    "soil_d_n_alb",
    "soil_texture",
    "soil_vf_clay",
    "soil_wf_clay",
    "soil_wf_om",
];

/// 站点文件缺哪些必需字段。
pub fn missing_fields(file: &Path) -> Result<Vec<String>> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    Ok(REQUIRED_FIELDS
        .iter()
        .filter(|n| f.variable(n).is_none())
        .map(|n| (*n).to_string())
        .collect())
}

/// 补齐一个站点文件。`rawdata` 为 `None` 时用模块默认值，并在 `source` 里说明。
pub fn fill(src: &Path, dst: &Path, rawdata: Option<&Path>) -> Result<Report> {
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;

    let (lon, lat, landtype, col, soil_dim) = read_inputs(dst)?;
    let d = derive(&col);
    let fe = fine_earth_fractions(&col);
    let texture = classify(fe.silt, fe.clay).with_context(|| {
        format!(
            "sand {:.2} silt {:.2} clay {:.2} is outside the USDA triangle",
            fe.sand, fe.silt, fe.clay
        )
    })?;

    let mut f =
        netcdf::append(dst).with_context(|| format!("cannot append to {}", dst.display()))?;

    let mut report = Report {
        texture,
        texture_name: CLASS_NAMES[(texture - 1) as usize].to_string(),
        bvic: BVIC_USDA[texture as usize],
        fine_earth: (fe.sand, fe.silt, fe.clay),
        from_raster: Vec::new(),
        from_default: Vec::new(),
    };

    // --- 栅格来源的 8 个 ---
    let (isc, lake, elev, elvstd, slope) = match rawdata {
        Some(r) => (
            point_i32(&r.join("soil_brightness.nc"), "soil_brightness", lon, lat).ok(),
            point_f64(&r.join("lake_depth.nc"), "lake_depth", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "elevation", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "elvstd", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "slope", lon, lat).ok(),
        ),
        None => (None, None, None, None, None),
    };

    // 有栅格就用栅格的颜色档；没有就退到标称档 10（1..=20 的中位），
    // 并如实标注。先前的脚本正是把 10 写死了 —— 错的不是这个数，而是把它
    // 当成实测值，且不管站点在哪都用它：实测 90 个站点里只有 1 个是 10。
    const NOMINAL_ISC: i32 = 10;
    let (use_isc, measured) = match isc {
        Some(i) => (i, true),
        None => (NOMINAL_ISC, false),
    };
    let a = albedo(use_isc, landtype).with_context(|| {
        format!(
            "no soil albedo for colour class {use_isc} and IGBP land type {landtype}; \
             CoLM leaves these at spval for water and ice, which this crate will not write silently"
        )
    })?;
    let src = if measured {
        format!("rawdata soil_brightness.nc colour class {use_isc}")
    } else {
        format!("synthesized: nominal soil colour class {use_isc} (mid-range); no soil_brightness raster given")
    };
    for (name, v) in [
        ("soil_s_v_alb", a.s_v),
        ("soil_d_v_alb", a.d_v),
        ("soil_s_n_alb", a.s_n),
        ("soil_d_n_alb", a.d_n),
    ] {
        put_scalar(&mut f, name, v, &src)?;
        if measured {
            report.from_raster.push(name.to_string());
        } else {
            report.from_default.push(name.to_string());
        }
    }

    for (name, got, default, note) in [
        (
            "lakedepth",
            lake,
            1.0,
            "MOD_SingleSrfdata.F90:47 module default",
        ),
        (
            "elevation",
            elev,
            0.0,
            "MOD_SingleSrfdata.F90:87 module default",
        ),
        (
            "elvstd",
            elvstd,
            0.0,
            "MOD_SingleSrfdata.F90:88 module default",
        ),
        (
            "sloperatio",
            slope,
            0.0,
            "MOD_SingleSrfdata.F90:89 module default",
        ),
    ] {
        match got {
            Some(v) => {
                put_scalar(&mut f, name, v, "rawdata raster")?;
                report.from_raster.push(name.to_string());
            }
            None => {
                put_scalar(&mut f, name, default, &format!("synthesized: {note}"))?;
                report.from_default.push(name.to_string());
            }
        }
    }

    // --- 推导的 4 个 ---
    // 维度取自它们各自的来源变量，而不是按长度去猜：站点文件里
    // LAI_year=2 / month=12 / pft=2 / soil=10 / year=21，按长度找只是碰巧
    // 不重复，而 dimensions() 的迭代顺序并无保证。
    let note =
        "derived: clay is 25% of the remainder in its own basis (loam 1:3 clay:silt assumption)";
    put_layers(&mut f, "soil_vf_clay", &d.vf_clay, &soil_dim, note)?;
    put_layers(&mut f, "soil_wf_clay", &d.wf_clay, &soil_dim, note)?;
    put_layers(
        &mut f,
        "soil_wf_om",
        &d.wf_om,
        &soil_dim,
        "derived: OM_density / BD_all",
    )?;
    put_int(
        &mut f,
        "soil_texture",
        texture as i32,
        &format!(
            "derived: CoLM USDA triangle on 0-60cm depth-weighted sand {:.2}% / silt {:.2}% / clay {:.2}% -> class {} ({}), BVIC {}",
            fe.sand, fe.silt, fe.clay, texture, report.texture_name, report.bvic
        ),
    )?;

    Ok(report)
}

/// 一次补齐的结果，供命令行打印与测试断言。
#[derive(Debug, Clone)]
pub struct Report {
    pub texture: u8,
    pub texture_name: String,
    pub bvic: f64,
    pub fine_earth: (f64, f64, f64),
    pub from_raster: Vec<String>,
    pub from_default: Vec<String>,
}

fn read_inputs(file: &Path) -> Result<(f64, f64, i32, SoilColumn, String)> {
    let f = netcdf::open(file)?;
    let scalar = |n: &str| -> Result<f64> {
        let v = f.variable(n).with_context(|| format!("{n} missing"))?;
        let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        x.first().copied().with_context(|| format!("{n} is empty"))
    };
    let layers = |n: &str| -> Result<Vec<f64>> {
        let v = f.variable(n).with_context(|| format!("{n} missing"))?;
        Ok(v.get_values(netcdf::Extents::All)?)
    };
    let lon = scalar("longitude")?;
    let lat = scalar("latitude")?;
    let landtype = scalar("IGBP_classification")? as i32;
    let col = SoilColumn {
        vf_sand: layers("soil_vf_sand")?,
        vf_gravels: layers("soil_vf_gravels")?,
        vf_om: layers("soil_vf_om")?,
        wf_sand: layers("soil_wf_sand")?,
        om_density: layers("soil_OM_density")?,
        bd_all: layers("soil_BD_all")?,
    };
    // 推导出来的剖面变量要挂在与来源变量同一个维度上。
    let soil_dim = f
        .variable("soil_vf_sand")
        .and_then(|v| v.dimensions().first().map(|d| d.name()))
        .context("soil_vf_sand has no dimension to hang the derived layers on")?;
    Ok((lon, lat, landtype, col, soil_dim))
}

fn put_scalar(f: &mut netcdf::FileMut, name: &str, value: f64, source: &str) -> Result<()> {
    let mut v = f.add_variable::<f64>(name, &[])?;
    v.put_values(&[value], netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

fn put_int(f: &mut netcdf::FileMut, name: &str, value: i32, source: &str) -> Result<()> {
    let mut v = f.add_variable::<i32>(name, &[])?;
    v.put_values(&[value], netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

fn put_layers(
    f: &mut netcdf::FileMut,
    name: &str,
    values: &[f64],
    dim: &str,
    source: &str,
) -> Result<()> {
    let mut v = f.add_variable::<f64>(name, &[dim])?;
    v.put_values(values, netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

#[cfg(test)]
#[path = "site_tests.rs"]
mod site_tests;
