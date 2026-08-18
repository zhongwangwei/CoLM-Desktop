//! 读一个 PLUMBER2 站点文件，补齐 12 个字段，写出增广站点文件。
//!
//! 做法是「拷贝后追加」而不是重建：站点文件里那 39 个变量连同它们的属性、
//! 维度、压缩设置都必须原样保留，重建一份等于把上游数据重新表述一遍，
//! 而任何一处表述差异都会变成一个没人发现的数值差异。
//!
//! 每个补进去的变量都带一个 `source` 属性，写明它是量出来的还是假设的。

use std::path::Path;

use anyhow::{bail, Context, Result};

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

/// 站点的身份：位置与地类。
///
/// 这三项 PLUMBER2 的站点文件自带，实测 CN-Cng 给出
/// `longitude = 123.5092` / `latitude = 44.5933` / `IGBP_classification = 10`，
/// 与手写算例里的 `SITE_lon_location` / `SITE_lat_location` / `SITE_landtype`
/// **逐位吻合**。所以新建算例时不该问用户要这三个数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub lon: f64,
    pub lat: f64,
    /// IGBP 分类号，直接对应 `SITE_landtype`。城市站点文件不带它，故为 `Option`。
    pub landtype: Option<i32>,
}

pub fn location(file: &Path) -> Result<Location> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    // 取全部值再拿第一个，而不是按标量读：PLUMBER2 的 `longitude` 是 0 维标量，
    // 而 Urban-PLUMBER 的是 `(y, x)`（各长 1）。按标量读后者会报
    // 「requested dimension (0) is bigger than the dimension length (2)」。
    // 两种形状都只描述一个站点，所以第一个值就是答案。
    let first = |name: &str| -> Result<Option<f64>> {
        let Some(v) = f.variable(name) else {
            return Ok(None);
        };
        Ok(v.get_values::<f64, _>(..)?.first().copied())
    };
    let need = |name: &str| -> Result<f64> {
        first(name)?.with_context(|| format!("{} has no {name}", file.display()))
    };
    Ok(Location {
        lon: need("longitude")?,
        lat: need("latitude")?,
        // 城市站点文件不带这一项 —— Urban-PLUMBER 的 21 个站一个都没有，
        // 而 CoLM 的 URBAN 路径反正会把地类强制成 13
        // （`MOD_SingleSrfdata.F90:1548`）。所以缺了不是错，是「这份文件不说」。
        landtype: first("IGBP_classification")?.map(|x| x as i32),
    })
}

/// 补齐一个站点文件。
///
/// 取值优先级是**站点自有 > 栅格 > 模块默认**。「站点自有」指站点文件本身的
/// 土壤剖面，以及 `observation` 指向的同站 `*_Flux.nc` 里的站点元数据 ——
/// 那里的 `elevation` 的 `long_name` 正是 "Site elevation"，90 个站点全都有。
/// 栅格是全球产品；站点自己有数的地方不该被它顶掉。
pub fn fill(
    src: &Path,
    dst: &Path,
    rawdata: Option<&Path>,
    observation: Option<&Path>,
) -> Result<Report> {
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;

    let (lon, lat, landtype, col, soil_dim) = read_inputs(dst)?;
    let d = derive(&col);
    let fe = fine_earth_fractions(&col);

    // --- 站点自己有的 ---
    // 质地类别由站点文件自己的土壤剖面算得（`classify` 在输入落到 USDA 三角外
    // 时返回 None）；高程取自同站 Observation 文件的 "Site elevation"。
    let site_texture = classify(fe.silt, fe.clay);
    let site_elevation = observation.and_then(|o| read_site_elevation(o).ok());

    // --- CoLM 的全球栅格 ---
    let raster_texture = rawdata.and_then(|r| {
        point_i32(
            &r.join("soil/soiltexture_0cm-60cm_mean.nc"),
            "soiltexture",
            lon,
            lat,
        )
        .ok()
        .filter(|t| (1..=12).contains(t))
        .map(|t| t as u8)
    });
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

    let (texture, texture_src) = resolve(site_texture, raster_texture, None).with_context(|| {
        format!(
            "sand {:.2} silt {:.2} clay {:.2} is outside the USDA triangle and no texture raster is available",
            fe.sand, fe.silt, fe.clay
        )
    })?;

    let mut f =
        netcdf::append(dst).with_context(|| format!("cannot append to {}", dst.display()))?;

    let mut report = Report {
        texture,
        site_texture,
        raster_texture,
        texture_name: CLASS_NAMES[(texture - 1) as usize].to_string(),
        bvic: BVIC_USDA[texture as usize],
        fine_earth: (fe.sand, fe.silt, fe.clay),
        from_site: Vec::new(),
        from_raster: Vec::new(),
        from_default: Vec::new(),
    };

    // --- 四个土壤反照率：站点侧没有对应值，所以只有栅格与标称档两级 ---
    // 标称档取 1..=20 的中位。先前的脚本正是把 10 写死了 —— 错的不是这个数，
    // 而是把它当成实测值且不管站点在哪都用它：实测 90 个站点里只有 1 个是 10。
    const NOMINAL_ISC: i32 = 10;
    let (use_isc, isc_src) = resolve(None, isc, Some(NOMINAL_ISC)).expect("has a fallback");
    let a = albedo(use_isc, landtype).with_context(|| {
        format!(
            "no soil albedo for colour class {use_isc} and IGBP land type {landtype}; \
             CoLM leaves these at spval for water and ice, which this crate will not write silently"
        )
    })?;
    let alb_note = match isc_src {
        Source::Raster => format!("rawdata soil_brightness.nc colour class {use_isc}"),
        _ => format!(
            "synthesized: nominal soil colour class {use_isc} (mid-range); no soil_brightness raster given"
        ),
    };
    for (name, v) in [
        ("soil_s_v_alb", a.s_v),
        ("soil_d_v_alb", a.d_v),
        ("soil_s_n_alb", a.s_n),
        ("soil_d_n_alb", a.d_n),
    ] {
        put_scalar(&mut f, name, v, &alb_note)?;
        report.record(name, isc_src);
    }

    // --- 标量字段：每一个都走同一条优先级 ---
    for (name, site, site_note, raster, fallback, fallback_note) in [
        (
            "elevation",
            site_elevation,
            "site: Site elevation from the Observation file",
            elev,
            0.0,
            "MOD_SingleSrfdata.F90:87 module default",
        ),
        (
            "lakedepth",
            None,
            "",
            lake,
            1.0,
            "MOD_SingleSrfdata.F90:47 module default",
        ),
        (
            "elvstd",
            None,
            "",
            elvstd,
            0.0,
            "MOD_SingleSrfdata.F90:88 module default",
        ),
        (
            "sloperatio",
            None,
            "",
            slope,
            0.0,
            "MOD_SingleSrfdata.F90:89 module default",
        ),
    ] {
        let (v, src) = resolve(site, raster, Some(fallback)).expect("has a fallback");
        let note = match src {
            Source::Site => site_note.to_string(),
            Source::Raster => "rawdata raster".to_string(),
            Source::Default => format!("synthesized: {fallback_note}"),
        };
        put_scalar(&mut f, name, v, &note)?;
        report.record(name, src);
    }

    // --- 由站点文件自己的土壤剖面推导的三个 ---
    // 维度取自它们各自的来源变量，而不是按长度去猜：站点文件里
    // LAI_year=2 / month=12 / pft=2 / soil=10 / year=21，按长度找只是碰巧
    // 不重复，而 dimensions() 的迭代顺序并无保证。
    let clay_note =
        "site: clay is 25% of the remainder in its own basis (loam 1:3 clay:silt assumption)";
    put_layers(&mut f, "soil_vf_clay", &d.vf_clay, &soil_dim, clay_note)?;
    put_layers(&mut f, "soil_wf_clay", &d.wf_clay, &soil_dim, clay_note)?;
    put_layers(
        &mut f,
        "soil_wf_om",
        &d.wf_om,
        &soil_dim,
        "site: OM_density / BD_all",
    )?;
    for name in ["soil_vf_clay", "soil_wf_clay", "soil_wf_om"] {
        report.record(name, Source::Site);
    }

    let texture_note = match texture_src {
        Source::Site => format!(
            "site: CoLM USDA triangle on this site's own 0-60cm depth-weighted sand {:.2}% / silt {:.2}% / clay {:.2}% (clay is an assumption) -> class {} ({}), BVIC {}",
            fe.sand, fe.silt, fe.clay, texture, report.texture_name, report.bvic
        ),
        _ => format!(
            "rawdata soil/soiltexture_0cm-60cm_mean.nc -> class {} ({}), BVIC {}; the site's own soil fell outside the USDA triangle",
            texture, report.texture_name, report.bvic
        ),
    };
    put_int(&mut f, "soil_texture", texture as i32, &texture_note)?;
    report.record("soil_texture", texture_src);

    Ok(report)
}

/// 一个字段的取值来源。**优先级就是这几个变体的顺序。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 站点自己有的：站点文件的土壤剖面，或同站 Observation 文件的站点元数据。
    Site,
    /// CoLM 的全球栅格。
    Raster,
    /// CoLM 的模块默认值。站点与栅格都没有时才用。
    Default,
}

/// 站点自有 > 栅格 > 模块默认。
///
/// 这条规则只写这一次，12 个字段全从这里走。先前每个字段各写各的分支，
/// 同一条规则被写成了四个形状 —— 那样规则就不在代码里，只在读代码的人脑子里。
///
/// `fallback` 为 `None` 表示这个字段没有兜底值，站点与栅格都拿不到就是错误。
fn resolve<T>(site: Option<T>, raster: Option<T>, fallback: Option<T>) -> Option<(T, Source)> {
    site.map(|v| (v, Source::Site))
        .or_else(|| raster.map(|v| (v, Source::Raster)))
        .or_else(|| fallback.map(|v| (v, Source::Default)))
}

/// 一次补齐的结果，供命令行打印与测试断言。
#[derive(Debug, Clone)]
pub struct Report {
    pub texture: u8,
    /// 分类器给出的类别；输入落到 USDA 三角外时为 `None`。
    /// 站点自己的土壤剖面算出的类别。落到 USDA 三角外时为 `None`，那时才退到栅格。
    pub site_texture: Option<u8>,
    /// CoLM 栅格给出的类别（若可读）。与 `texture` 不同是常态：
    /// 实测 90 个站点里两者只有 26 个一致，因为出自不同的土壤产品。
    pub raster_texture: Option<u8>,
    pub texture_name: String,
    pub bvic: f64,
    pub fine_earth: (f64, f64, f64),
    /// 取自站点自有数据的字段。
    pub from_site: Vec<String>,
    pub from_raster: Vec<String>,
    pub from_default: Vec<String>,
}

impl Report {
    fn record(&mut self, name: &str, src: Source) {
        match src {
            Source::Site => self.from_site.push(name.to_string()),
            Source::Raster => self.from_raster.push(name.to_string()),
            Source::Default => self.from_default.push(name.to_string()),
        }
    }
}

/// 同站 `*_Flux.nc` 里的 "Site elevation"。
///
/// 这是站点自己的元数据，不是全球产品插值 —— 90 个 PLUMBER2 站点全都带它，
/// 所以站点有数时它应当压过地形栅格。
fn read_site_elevation(obs: &Path) -> Result<f64> {
    let f = netcdf::open(obs).with_context(|| format!("cannot open {}", obs.display()))?;
    let v = f
        .variable("elevation")
        .with_context(|| format!("no elevation in {}", obs.display()))?;
    let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
    let e = x
        .first()
        .copied()
        .with_context(|| format!("elevation is empty in {}", obs.display()))?;
    if !e.is_finite() || e <= -9000.0 {
        bail!("elevation in {} is a fill value ({e})", obs.display());
    }
    Ok(e)
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

/// 补齐一个**城市**站点文件（Urban-PLUMBER 形状）。
///
/// 与 `fill` 是两件事，所以是两个函数。`fill` 服务 PLUMBER2：那里的活是
/// 补 12 个缺失字段，要土壤剖面、要 USDA 三角、要栅格。城市站点文件的
/// 变量集完全不同（23 个城市形态学量，没有土壤剖面也没有
/// `IGBP_classification`），而 CoLM 的 URBAN 路径本来就直接读原件 ——
/// 实测那份跑成功过的示例算例用的就是未经处理的 Urban-PLUMBER 原文件，
/// 与仓库里的原件逐字节相同。
///
/// 所以这里只做**一件**事：把 `ground_height` 也写成 `elevation`。
///
/// 为什么值得做：CoLM 的 URBAN 路径在站点文件没有 `elevation` 时回落到
/// `<rawdata>/elevation.nc`，那是个 **7 GB** 的全球栅格，而桌面用户装不了。
/// 同一段代码里 `elvstd` 与 `sloperatio` 却回落到 `topography.nc`
/// （`MOD_SingleSrfdata.F90:2496-2527`）—— CoLM 自己的不一致，不是我们能改的。
///
/// 为什么这个改名有依据而不是猜：`ground_height` 的属性写着
/// `long_name = "Ground height above sea level"`、`units = "m"`，
/// 与 CoLM 的 `SITE_elevation` 是同一个量。
pub fn prepare_urban(src: &Path, dst: &Path) -> Result<UrbanReport> {
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;

    let existing = {
        let f = netcdf::open(dst)?;
        (
            f.variable("elevation").is_some(),
            f.variable("ground_height").is_some(),
        )
    };
    match existing {
        // 已经有了就不动 —— 站点文件自己说的话优先。
        (true, _) => Ok(UrbanReport { elevation: None }),
        (false, false) => Ok(UrbanReport { elevation: None }),
        (false, true) => {
            let h = {
                let f = netcdf::open(dst)?;
                let v = f.variable("ground_height").expect("checked above");
                v.get_values::<f64, _>(..)?
                    .first()
                    .copied()
                    .context("ground_height is empty")?
            };
            let mut f = netcdf::append(dst)
                .with_context(|| format!("cannot append to {}", dst.display()))?;
            put_scalar(
                &mut f,
                "elevation",
                h,
                "Urban-PLUMBER ground_height (ground height above sea level)",
            )?;
            Ok(UrbanReport { elevation: Some(h) })
        }
    }
}

/// `prepare_urban` 做了什么。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanReport {
    /// 从 `ground_height` 补进去的高程；`None` 表示没补（本来就有，或者没得补）。
    pub elevation: Option<f64>,
}
