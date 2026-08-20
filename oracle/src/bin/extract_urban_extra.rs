//! 把 21 个 Urban-PLUMBER 站点**剩下**的栅格点值抽出来，生成
//! `crates/colm-srfdata/src/urban_extra.rs`。
//!
//! 用法:
//!   cargo run -p oracle --bin extract-urban-extra -- <Sitedata> <rawdata> <lai.json> \
//!     > crates/colm-srfdata/src/urban_extra.rs
//!   cargo run -p oracle --bin extract-urban-extra -- <Sitedata> <rawdata> --from-tiles \
//!     > crates/colm-srfdata/src/urban_extra.rs
//!
//! `extract-urban-soil` 搬走的是 `soil/` 那 122 GB；这一支搬走剩下的六处，
//! 清单同样是**实测**出来的（拿真 rawdata 跑一遍城市算例，照 mksrfdata
//! 报的 `Warning: <name> not found in site.nc` 逐条对）：
//!
//! | 量 | 栅格 | CoLM 读它的地方 |
//! |---|---|---|
//! | `LCZ_DOM` | `urban_type/` 5x5 瓦片 | `MOD_SingleSrfdata.F90:1591` |
//! | `LUCY_ID` | `urban/LUCY_regionid.nc`（**colm_5km**） | :1862 |
//! | 土壤颜色档 | `soil_brightness.nc` | :2072 |
//! | `lakedepth` | `lake_depth.nc` | :2050 |
//! | `elvstd` / `sloperatio` | `topography.nc` | :2497 / :2508 |
//! | `TREE_LAI` / `TREE_SAI` | `urban_lai_500m/` 5x5 瓦片 | :1753 |
//!
//! **为什么树 LAI 也抽表而不是发瓦片**：21 个站落在 15 个 5x5 块里，年份
//! 2000-2022 共 23 年，而单个 URBLAI 瓦片实测 **85 MB**（1200x1200x12 的
//! double，两个变量）—— 发瓦片是 15 x 23 x 85 MB ≈ **7 GB**，抽表是
//! 21 x 23 x 12 x 2 = 11592 个数 ≈ 230 KB。差四个数量级。
//!
//! **树 LAI 默认从一份点值 JSON 读，而不是从瓦片读。** `urban_lai_500m/`
//! 是一块 SMB 网络盘上的 698 GB 目录，盘不在就整支工具都跑不了 ——
//! 而其余六个来源都是本地文件。JSON 的结构是
//! `{站点: {lon, lat, block, years: {年: {URBAN_TREE_LAI: [12], URBAN_TREE_SAI: [12]}}}}`，
//! 里面的 `block` 会与本工具自己算出来的 5x5 瓦片名对照 —— 两条独立算出来的
//! 路径对不上就停下来，那正是 `get_5x5_filename` 那串 `(19-jbox)*5` 最容易
//! 错的地方。盘挂着时传 `--from-tiles` 可以绕过 JSON 直接重读瓦片。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use colm_srfdata::grid::COLM_5KM;
use colm_srfdata::raster::{
    point_5x5_i32, point_5x5_time_f64, point_f64, point_f64_on, point_i32, tile_5x5_path,
};
use oracle::sitedata::{urban_sites, Site};

/// 城市树 LAI/SAI 的月份数。CoLM 那边写死 `ntime = 12`
/// （`MOD_SingleSrfdata.F90:1741`）。
const NMONTH: usize = 12;

/// 一个站点抽出来的全部东西。
struct Row {
    name: String,
    lon: f64,
    lat: f64,
    lcz_dom: i32,
    lucy_id: f64,
    soil_colour: i32,
    lakedepth: f64,
    elvstd: f64,
    sloperatio: f64,
    /// `[年][月]`。第一维与 `LAI_YEARS` 对齐。
    tree_lai: Vec<[f64; NMONTH]>,
    tree_sai: Vec<[f64; NMONTH]>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [sitedata, rawdata, lai_source] = args.as_slice() else {
        bail!(
            "usage: extract-urban-extra <Urban-PLUMBER/Sitedata> <rawdata> \
             <lai-points.json>|--from-tiles"
        );
    };
    let sitedata = Path::new(sitedata);
    let raw = Path::new(rawdata);

    let sites = urban_sites(sitedata)?;
    if sites.is_empty() {
        bail!("no *_site_v1.nc under {}", sitedata.display());
    }

    // **年份集合要先定下来，而且必须 21 个站一致。** 表里只留一份
    // `LAI_YEARS`，各站年份不同的话这个形状就是错的 —— 与其让它悄悄错位，
    // 不如在这里停下来。
    let lai = if lai_source == "--from-tiles" {
        LaiSource::Tiles
    } else {
        LaiSource::Points(read_lai_points(Path::new(lai_source), &sites, raw)?)
    };
    let years = match &lai {
        LaiSource::Tiles => common_lai_years(&sites, raw)?,
        LaiSource::Points(p) => common_years(p)?,
    };

    let mut rows = Vec::new();
    for s in &sites {
        rows.push(extract(s, raw, &years, &lai)?);
    }

    emit(&rows, &years);
    Ok(())
}

/// 树 LAI/SAI 从哪来。
enum LaiSource {
    /// 直接读 `urban_lai_500m/` 的 5x5 瓦片。要那块网络盘挂着。
    Tiles,
    /// 一份预先抽好的点值：站点 -> 年 -> (12 个月 LAI, 12 个月 SAI)。
    Points(Points),
}

type Points = BTreeMap<String, BTreeMap<i32, ([f64; NMONTH], [f64; NMONTH])>>;

/// 读点值 JSON，并对着站点文件与 5x5 命名逐条核对。
///
/// 核三样：站点集合一致、经纬度与站点文件一致（1e-3 内，与 `lookup` 同一个
/// 判据）、`block` 与本工具从 `get_5x5_filename` 算出来的瓦片名一致。
/// 第三条是白送的交叉校验 —— 那串 `(19-jbox)*5` / `(18-jbox)*5` 写反了
/// 会得到一个**存在的**、但差 5 度的文件名，而两条独立算法同时写反的
/// 可能性远小于一条。
fn read_lai_points(file: &Path, sites: &[Site], raw: &Path) -> Result<Points> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("{} is not JSON", file.display()))?;
    let obj = doc
        .as_object()
        .with_context(|| format!("{} is not a JSON object", file.display()))?;

    let mut out: Points = BTreeMap::new();
    for s in sites {
        let node = obj
            .get(&s.name)
            .with_context(|| format!("{} has no entry for {}", file.display(), s.name))?;
        let num = |k: &str| -> Result<f64> {
            node.get(k)
                .and_then(|v| v.as_f64())
                .with_context(|| format!("{}: {k} is missing or not a number", s.name))
        };
        let (lon, lat) = (num("lon")?, num("lat")?);
        if (lon - s.lon).abs() >= 1e-3 || (lat - s.lat).abs() >= 1e-3 {
            bail!(
                "{}: the LAI points say ({lon}, {lat}) but {} says ({}, {})",
                s.name,
                s.file.display(),
                s.lon,
                s.lat
            );
        }
        let block = node
            .get("block")
            .and_then(|v| v.as_str())
            .with_context(|| format!("{}: no block name", s.name))?;
        let (probe, _, _) = tile_5x5_path(&raw.join("urban_lai_500m"), "URBLAI_0000", lon, lat);
        let mine = probe
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split_once(".URBLAI_"))
            .map(|(a, _)| a)
            .context("cannot derive the 5x5 tile stem")?;
        if mine != block {
            bail!(
                "{}: the LAI points came from {block} but get_5x5_filename gives {mine}; \
                 one of the two 5x5 namings is wrong",
                s.name
            );
        }

        let years = node
            .get("years")
            .and_then(|v| v.as_object())
            .with_context(|| format!("{}: no years", s.name))?;
        let mut per_year = BTreeMap::new();
        for (y, node) in years {
            let y: i32 = y
                .parse()
                .with_context(|| format!("{}: {y:?} is not a year", s.name))?;
            let months = |k: &str| -> Result<[f64; NMONTH]> {
                let xs = node
                    .get(k)
                    .and_then(|v| v.as_array())
                    .with_context(|| format!("{}: {y} has no {k}", s.name))?;
                if xs.len() != NMONTH {
                    bail!("{}: {y} {k} has {} months, not {NMONTH}", s.name, xs.len());
                }
                let mut out = [0.0f64; NMONTH];
                for (slot, v) in out.iter_mut().zip(xs) {
                    let x = v
                        .as_f64()
                        .with_context(|| format!("{}: {y} {k} has a non-number", s.name))?;
                    if !x.is_finite() {
                        bail!("{}: {y} {k} is {x}, which cannot be a Rust literal", s.name);
                    }
                    *slot = x;
                }
                Ok(out)
            };
            // 瓦片里叫 `URBAN_TREE_LAI`/`URBAN_TREE_SAI`；site.nc 里 CoLM 要的
            // 是 `TREE_LAI`/`TREE_SAI`。改名发生在写站点文件那一步。
            per_year.insert(y, (months("URBAN_TREE_LAI")?, months("URBAN_TREE_SAI")?));
        }
        out.insert(s.name.clone(), per_year);
    }
    Ok(out)
}

/// 点值表里各站的年份集合，确认一致后返回。
fn common_years(points: &Points) -> Result<Vec<i32>> {
    let mut common: Option<(&str, BTreeSet<i32>)> = None;
    for (name, per_year) in points {
        let have: BTreeSet<i32> = per_year.keys().copied().collect();
        match &common {
            None => common = Some((name, have)),
            Some((first, c)) if *c != have => bail!(
                "{name} covers {have:?} but {first} covers {c:?}; one shared LAI_YEARS cannot \
                 describe both, so the generated table needs a per-site year list"
            ),
            Some(_) => {}
        }
    }
    let (_, years) = common.context("the LAI points table is empty")?;
    Ok(years.into_iter().collect())
}

/// 各站的 URBLAI 年份集合，确认一致后返回。
///
/// 年份不从常量里写死，而是**照着瓦片目录数出来**：换一棵 rawdata 之后
/// 年份跟着变，写死的那个数不会跟着变，而它错了没有任何症状 —— 只是
/// 某一年的树 LAI 悄悄用了另一年的。
fn common_lai_years(sites: &[Site], raw: &Path) -> Result<Vec<i32>> {
    let dir = raw.join("urban_lai_500m");
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let mut common: Option<BTreeSet<i32>> = None;
    let mut first = String::new();
    for s in sites {
        let (probe, _, _) = tile_5x5_path(&dir, "URBLAI_0000", s.lon, s.lat);
        let stem = probe
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split_once(".URBLAI_"))
            .map(|(a, _)| a.to_string())
            .context("cannot derive the 5x5 tile stem")?;
        let mut have = BTreeSet::new();
        for entry in std::fs::read_dir(&dir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name.strip_prefix(&format!("{stem}.URBLAI_")) {
                if let Some(y) = rest.strip_suffix(".nc").and_then(|y| y.parse::<i32>().ok()) {
                    have.insert(y);
                }
            }
        }
        if have.is_empty() {
            bail!("{}: no {stem}.URBLAI_*.nc under {}", s.name, dir.display());
        }
        match &common {
            None => {
                common = Some(have);
                first = s.name.clone();
            }
            Some(c) if *c != have => bail!(
                "{} covers {:?} but {first} covers {:?}; one shared LAI_YEARS cannot describe both, \
                 so the generated table needs a per-site year list",
                s.name,
                have,
                c
            ),
            Some(_) => {}
        }
    }
    Ok(common.expect("at least one site").into_iter().collect())
}

fn extract(s: &Site, raw: &Path, years: &[i32], lai: &LaiSource) -> Result<Row> {
    let (name, lon, lat) = (&s.name, s.lon, s.lat);
    let at = |what: &str| format!("{name}: reading {what}");

    // LCZ 局地气候区，`urban_type/` 的 5x5 瓦片。整型，`_FillValue = 0`。
    let lcz_dom = point_5x5_i32(&raw.join("urban_type"), "URBTYP", "LCZ_DOM", lon, lat)
        .with_context(|| at("LCZ_DOM"))?;

    // LUCY 区号。**这个栅格是 colm_5km，不是 colm_500m** —— 见
    // `MOD_SingleSrfdata.F90:1861`。而且 CoLM 用 `read_point_var_2d_real8`
    // 读一个 int 变量，所以站点文件里也按实型写。
    let lucy_id = point_f64_on(
        COLM_5KM,
        &raw.join("urban/LUCY_regionid.nc"),
        "LUCY_REGION_ID",
        lon,
        lat,
    )
    .with_context(|| at("LUCY_REGION_ID"))?;

    // 土壤颜色档。四个反照率不在这里算 —— 存档位，写站点文件时再过
    // `MOD_SoilColorRefl.F90` 那张表（`colm_srfdata::albedo`）。
    // 存档位而不是存四个数：档位才是量出来的，四个数是 CoLM 自己的常量表。
    let soil_colour = point_i32(&raw.join("soil_brightness.nc"), "soil_brightness", lon, lat)
        .with_context(|| at("soil_brightness"))?;

    // 湖深。**要乘 0.1**：CoLM 是 `SITE_lakedepth = lakedepth * 0.1`
    // （`MOD_SingleSrfdata.F90:2052`），而站点文件那条路径直接就把读到的数
    // 当成 `SITE_lakedepth`。存原始值会让湖深大十倍。
    let lakedepth = point_f64(&raw.join("lake_depth.nc"), "lake_depth", lon, lat)
        .with_context(|| at("lake_depth"))?
        * 0.1;

    let topo = raw.join("topography.nc");
    let elvstd = point_f64(&topo, "elvstd", lon, lat).with_context(|| at("elvstd"))?;
    // 站点文件里叫 `sloperatio`，栅格里叫 `slope`。
    let sloperatio = point_f64(&topo, "slope", lon, lat).with_context(|| at("slope"))?;

    // 城市树 LAI/SAI：每年一个瓦片，每个瓦片 12 个月。
    let (mut tree_lai, mut tree_sai) = (Vec::new(), Vec::new());
    for y in years {
        let (l, s2) = match lai {
            LaiSource::Points(p) => *p
                .get(name)
                .and_then(|per_year| per_year.get(y))
                .with_context(|| at(&format!("tree LAI for {y}")))?,
            LaiSource::Tiles => {
                let dir = raw.join("urban_lai_500m");
                let sfx = format!("URBLAI_{y}");
                let mut l = [0.0f64; NMONTH];
                let mut s2 = [0.0f64; NMONTH];
                for m in 0..NMONTH {
                    // itime 是 1-based，与 Fortran 的 `DO itime = 1, ntime` 一致。
                    l[m] = point_5x5_time_f64(&dir, &sfx, "URBAN_TREE_LAI", lon, lat, m + 1)
                        .with_context(|| at(&format!("URBAN_TREE_LAI {y}-{:02}", m + 1)))?;
                    s2[m] = point_5x5_time_f64(&dir, &sfx, "URBAN_TREE_SAI", lon, lat, m + 1)
                        .with_context(|| at(&format!("URBAN_TREE_SAI {y}-{:02}", m + 1)))?;
                }
                (l, s2)
            }
        };
        tree_lai.push(l);
        tree_sai.push(s2);
    }

    for (what, x) in [
        ("lucy_id", lucy_id),
        ("lakedepth", lakedepth),
        ("elvstd", elvstd),
        ("sloperatio", sloperatio),
    ] {
        if !x.is_finite() {
            bail!("{name}: {what} is {x}, which cannot be written as a Rust literal");
        }
    }

    Ok(Row {
        name: name.clone(),
        lon,
        lat,
        lcz_dom,
        lucy_id,
        soil_colour,
        lakedepth,
        elvstd,
        sloperatio,
        tree_lai,
        tree_sai,
    })
}

fn emit(rows: &[Row], years: &[i32]) {
    let ny = years.len();
    print!("{HEADER}");

    println!("/// URBLAI 瓦片覆盖的年份，升序。**21 个站一致**，抽取时校验过。");
    println!("///");
    println!("/// 写进 site.nc 的 `LAI_year` 就是它。CoLM 运行时按");
    println!(
        "/// `findloc_ud(SITE_LAI_year == min(DEF_LAI_END_YEAR, max(DEF_LAI_START_YEAR, year)))`"
    );
    println!("/// 取下标（`MOD_Urban_LAIReadin.F90:58`），所以只要这张表盖住");
    println!("/// `DEF_LAI_START_YEAR..=DEF_LAI_END_YEAR`（默认 2000..=2020），");
    println!("/// 任何模拟年都能落到与「让 CoLM 自己去开瓦片」同一年上。");
    println!("#[rustfmt::skip]");
    println!("pub static LAI_YEARS: [i32; {ny}] = [");
    println!(
        "    {},",
        years
            .iter()
            .map(|y| y.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("];");
    println!();

    println!("/// 一个城市站点的第二批栅格点值。");
    println!("///");
    println!("/// 与 [`crate::urban_soil::UrbanSoil`] 是两张表而不是一张：那张是");
    println!("/// `soil/` 的剖面，这张是六个各自独立的栅格。合成一张的话，重抽其中");
    println!("/// 一半就得把另一半也重抽一遍。");
    println!("pub struct UrbanExtra {{");
    println!("    pub site: &'static str,");
    println!("    pub lon: f64,");
    println!("    pub lat: f64,");
    println!("    /// 局地气候区分类，写进 site.nc 的 `LCZ_DOM`。");
    println!("    /// 实测 21 个站落在 7 个类别上 —— 编一个默认值会把大多数站换掉。");
    println!("    pub lcz_dom: i32,");
    println!("    /// LUCY 区号，写进 site.nc 的 `LUCY_ID`。");
    println!("    /// CoLM 按实型读（`read_point_var_2d_real8`），所以这里也是 f64。");
    println!("    pub lucy_id: f64,");
    println!("    /// 土壤颜色档 1..=20。**四个反照率不入表** —— 它们由");
    println!("    /// [`crate::albedo::albedo`] 从这个档位查 CoLM 自己的常量表得到，");
    println!("    /// 量出来的是档位，查表是 CoLM 的算法。");
    println!("    pub soil_colour: i32,");
    println!("    /// 湖深，**已经乘过 0.1**，与 `SITE_lakedepth` 同义。");
    println!("    /// 实测 21 个站全是 0.0，而模块默认值是 1.0。");
    println!("    pub lakedepth: f64,");
    println!("    pub elvstd: f64,");
    println!("    /// 站点文件里叫 `sloperatio`，栅格里叫 `slope`。");
    println!("    pub sloperatio: f64,");
    println!("    /// `[年][月]`，第一维与 [`LAI_YEARS`] 对齐。");
    println!("    pub tree_lai: [[f64; 12]; {ny}],");
    println!("    pub tree_sai: [[f64; 12]; {ny}],");
    println!("}}");
    println!();

    // 数据表不参与 rustfmt —— 11592 个数拆成几千行之后，重新抽一次数据的
    // diff 会从「哪几个站点变了」变成满屏噪音。`urban_soil.rs` 同理。
    println!("// **这张表不参与 rustfmt。** 每行是一个站点一个年份的 12 个月，");
    println!("// rustfmt 会把超宽的数组拆开 —— 11592 个数拆完之后，重抽一次数据的");
    println!("// diff 就没法读了。`urban_soil.rs` 出于同一个理由也带这个属性。");
    println!("#[rustfmt::skip]");
    println!("pub static SITES: &[UrbanExtra] = &[");
    for r in rows {
        println!("    UrbanExtra {{");
        println!("        site: {:?},", r.name);
        println!("        lon: {:?},", r.lon);
        println!("        lat: {:?},", r.lat);
        println!("        lcz_dom: {},", r.lcz_dom);
        println!("        lucy_id: {:?},", r.lucy_id);
        println!("        soil_colour: {},", r.soil_colour);
        println!("        lakedepth: {:?},", r.lakedepth);
        println!("        elvstd: {:?},", r.elvstd);
        println!("        sloperatio: {:?},", r.sloperatio);
        for (field, table) in [("tree_lai", &r.tree_lai), ("tree_sai", &r.tree_sai)] {
            println!("        {field}: [");
            for (y, months) in years.iter().zip(table.iter()) {
                let vals: Vec<String> = months.iter().map(|x| format!("{x:?}")).collect();
                println!("            /* {y} */ [{}],", vals.join(", "));
            }
            println!("        ],");
        }
        println!("    }},");
    }
    println!("];");
    print!("{LOOKUP}");
}

const HEADER: &str = r#"//! 21 个 Urban-PLUMBER 站点在**六个**全球栅格上的点值，从 CoLM 2024
//! rawdata 抽出。
//!
//! **生成的产物，不要手改。** 重生成：
//! `cargo run -p oracle --bin extract-urban-extra -- <Sitedata> <rawdata> <lai.json> > 本文件`
//! （树 LAI 的点值来自那份 JSON；`--from-tiles` 换成直接读 698 GB 的瓦片目录。）
//!
//! **为什么要它**：`urban_soil.rs` 搬走了 `soil/` 那 122 GB 之后，城市算例
//! 还有六处会去开栅格，而且**开不到就 `CoLM_stop`，不是警告**。其中
//! `urban_lai_500m/` 单个瓦片实测 85 MB，21 个站要 15 块 x 23 年 ≈ 7 GB；
//! 这张表把同一批数压到 230 KB。
//!
//! **这些值是量出来的，不是假设的**：来源是 CoLM 2024 rawdata 在该站点
//! 经纬度上的格点值，取点算法与 `share/MOD_NetCDFPoint.F90` 逐行对应。
//! 写进 site.nc 时的 `source` 属性要说出这一点。
//!
//! **查不到的站点一个字都不写。** 表只覆盖 Urban-PLUMBER 那 21 个站；
//! 表外的站点让 CoLM 照旧回落栅格。编一个 `LCZ_DOM` 出来，会把整个城市
//! 形态换掉而结果看上去仍然正常。
//!
//! **照抄栅格，不替它「修正」。** 两处看着可疑的地方都是原样入库的：
//! 南半球的 AU-Preston / AU-SurreyHills 抽出来的月相位像北半球物候
//! （1 月低、7 月高），而 FI-Torni 全年 0.00。前者是产品自身的性质，
//! 后者是赫尔辛基市中心的塔楼站、周边几乎没有树（同城的 FI-Kumpula 是
//! 1.6 左右）。改成「看着对」的值就不再与「让 CoLM 自己去读栅格」逐位
//! 相同了 —— 与 `urban_soil.rs` 照抄 `soil_texture = -1` 是同一条规矩。

"#;

const LOOKUP: &str = r#"
/// 按经纬度找这个站点的第二批点值。
///
/// 判据与 [`crate::urban_soil::lookup`] 完全一致 —— 两张表按同一个键对齐，
/// 一张命中而另一张没命中会是个说不清的状态。
pub fn lookup(lon: f64, lat: f64) -> Option<&'static UrbanExtra> {
    SITES
        .iter()
        .find(|s| (s.lon - lon).abs() < 1e-3 && (s.lat - lat).abs() < 1e-3)
}

#[cfg(test)]
#[path = "urban_extra_tests.rs"]
mod urban_extra_tests;
"#;

#[cfg(test)]
mod tests {
    /// serde_json 必须**逐位**还原 JSON 里的浮点数。
    ///
    /// 默认的 serde_json 做不到：它的快速路径在这个数上差 1 ULP
    /// （`1.8337343205163141` → `1.833734320516314`）。而这一位之差会
    /// 一路走进 site.nc、srfdata.nc，最后让 history 与「让 CoLM 自己去读
    /// 栅格」的参照运行差在第 16 位上 —— 实测 `f_tref` 有 161/264 个点不同。
    ///
    /// 修法是 `oracle/Cargo.toml` 里的 `features = ["float_roundtrip"]`。
    /// 这条测试钉住它：谁把那个 feature 去掉，这里当场变红，而不是等到
    /// 下一次重抽数据、再跑一次黄金比对时才发现。
    #[test]
    fn serde_json_parses_floats_exactly() {
        // 这个数是 AU-Preston 2000 年 1 月的树 LAI，出自参照运行的
        // srfdata.nc —— 正是踩中默认解析器那一位的那个值。
        for text in [
            "1.8337343205163141",
            "1.8957902952026717",
            "0.9811675359034915",
        ] {
            let exact: f64 = text.parse().expect("Rust 自己的解析是正确舍入的");
            let v: serde_json::Value =
                serde_json::from_str(&format!("[{text}]")).expect("不是 JSON");
            let got = v[0].as_f64().expect("不是数");
            assert_eq!(
                got.to_bits(),
                exact.to_bits(),
                "serde_json 把 {text} 解析成了 {got:?}，与 {exact:?} 差 {} ULP",
                (got.to_bits() as i64 - exact.to_bits() as i64).abs()
            );
        }
    }
}
