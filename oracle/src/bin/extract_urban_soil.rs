//! 把 21 个 Urban-PLUMBER 站点的土壤剖面点值从全球栅格里抽出来，生成
//! `crates/colm-srfdata/src/urban_soil.rs`。
//!
//! 用法:
//!   cargo run -p oracle --bin extract-urban-soil -- <Sitedata> <rawdata> \
//!     > crates/colm-srfdata/src/urban_soil.rs
//!
//! 清单是**实测**出来的，不是照 `MOD_SingleSrfdata.F90` 推的：拿一棵去掉
//! `soil/` 的 rawdata 软链树反复跑 `mksrfdata`，按它报的缺文件逐轮补，直到
//! 三段全绿。结论是 24 个剖面量（各 8 层）加一个标量 `soil_texture` ——
//! `soil_texture` 只在 `DEF_Runoff_SCHEME == 3`（**CoLM 的默认值**）下才读，
//! 照源码扫一眼很容易漏掉。
//!
//! 层数是 8：`mkinidata/MOD_SoilParametersReadin.F90` 是 `DO nsl = 1, 8`，
//! 而栅格每层一个独立变量，正好 8 个。城市段回落时也是 `allocate(...(8))`。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_srfdata::raster::{point_f64, point_i32};

/// 一个剖面量：Rust 字段名、site.nc 变量名、栅格文件名、栅格里的层变量前缀。
///
/// **四者的对应是不规则的，只能显式列**：`k_s.nc` → `soil_k_s`（`_s` 留着），
/// 而 `BD_all_s.nc` → `soil_BD_all`（`_s` 去掉）；`VGM_alpha.nc` 更是整个
/// 换了词序，对到 `soil_alpha_vgm`。按规则推必错。
/// 顺序即 `MOD_SingleSrfdata.F90` 城市段的读取顺序。
///
/// 保持一行一项：四列对齐着看才能一眼核出哪一列跟哪一列不一致，
/// 拆成每项六行就核不动了。
#[rustfmt::skip]
const PROFILE: [(&str, &str, &str, &str); 24] = [
    ("vf_quartz_mineral", "soil_vf_quartz_mineral", "vf_quartz_mineral_s.nc", "vf_quartz_mineral_s_l"),
    ("vf_gravels", "soil_vf_gravels", "vf_gravels_s.nc", "vf_gravels_s_l"),
    ("vf_sand", "soil_vf_sand", "vf_sand_s.nc", "vf_sand_s_l"),
    ("vf_clay", "soil_vf_clay", "vf_clay_s.nc", "vf_clay_s_l"),
    ("vf_om", "soil_vf_om", "vf_om_s.nc", "vf_om_s_l"),
    ("wf_gravels", "soil_wf_gravels", "wf_gravels_s.nc", "wf_gravels_s_l"),
    ("wf_sand", "soil_wf_sand", "wf_sand_s.nc", "wf_sand_s_l"),
    ("wf_clay", "soil_wf_clay", "wf_clay_s.nc", "wf_clay_s_l"),
    ("wf_om", "soil_wf_om", "wf_om_s.nc", "wf_om_s_l"),
    ("om_density", "soil_OM_density", "OM_density_s.nc", "OM_density_s_l"),
    ("bd_all", "soil_BD_all", "BD_all_s.nc", "BD_all_s_l"),
    ("theta_s", "soil_theta_s", "theta_s.nc", "theta_s_l"),
    ("k_s", "soil_k_s", "k_s.nc", "k_s_l"),
    ("csol", "soil_csol", "csol.nc", "csol_l"),
    ("tksatu", "soil_tksatu", "tksatu.nc", "tksatu_l"),
    ("tksatf", "soil_tksatf", "tksatf.nc", "tksatf_l"),
    ("tkdry", "soil_tkdry", "tkdry.nc", "tkdry_l"),
    ("k_solids", "soil_k_solids", "k_solids.nc", "k_solids_l"),
    ("psi_s", "soil_psi_s", "psi_s.nc", "psi_s_l"),
    ("lambda", "soil_lambda", "lambda.nc", "lambda_l"),
    ("theta_r", "soil_theta_r", "VGM_theta_r.nc", "VGM_theta_r_l"),
    ("alpha_vgm", "soil_alpha_vgm", "VGM_alpha.nc", "VGM_alpha_l"),
    ("l_vgm", "soil_L_vgm", "VGM_L.nc", "VGM_L_l"),
    ("n_vgm", "soil_n_vgm", "VGM_n.nc", "VGM_n_l"),
];

/// 质地栅格是整型、单层，且**在城市像元上大面积缺测**，所以单独处理。
const TEXTURE_FILE: &str = "soiltexture_0cm-60cm_mean.nc";
const TEXTURE_VAR: &str = "soiltexture";

/// 质地缺测时写进表里的值，与栅格自己的 `_FillValue` 一致。
///
/// 实测 21 个城市站点里有 16 个落在这个洞里 —— 土壤质地产品在建成区没有
/// 数据。这**不是**抽取失败：CoLM 拿到负值会 `WHERE (soiltext < 0) soiltext = 0`
/// （`mkinidata/MOD_SoilTextureReadin.F90`），再取 `BVIC_USDA(0) = 1.0`。
/// 照抄 -1 因此与「让 CoLM 自己去读那 122 GB」逐位一致，而换成由砂黏比
/// 反推一个类别反倒会**改掉**结果。
const TEXTURE_FILL: i32 = -1;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [sitedata, rawdata] = args.as_slice() else {
        bail!("usage: extract-urban-soil <Urban-PLUMBER/Sitedata> <rawdata>");
    };
    let sitedata = Path::new(sitedata);
    let soil = Path::new(rawdata).join("soil");
    if !soil.is_dir() {
        bail!("{} is not a directory", soil.display());
    }

    let sites = collect_sites(sitedata)?;
    if sites.is_empty() {
        bail!("no *_site_v1.nc under {}", sitedata.display());
    }

    let mut rows = Vec::new();
    for (name, file) in &sites {
        let (lon, lat) = read_lonlat(file)?;
        let mut profiles = Vec::new();
        for (_, _, raw_file, prefix) in PROFILE {
            let path = soil.join(raw_file);
            let mut layers = [0.0f64; 8];
            for (n, slot) in layers.iter_mut().enumerate() {
                let var = format!("{prefix}{}", n + 1);
                let x = point_f64(&path, &var, lon, lat)
                    .with_context(|| format!("{name}: reading {var} from {}", path.display()))?;
                if !x.is_finite() {
                    bail!("{name}: {var} is {x}, which cannot be written as a Rust literal");
                }
                *slot = x;
            }
            profiles.push(layers);
        }
        // 质地：缺测走 `TEXTURE_FILL`，其余错误（文件没了、变量改名）照常抛。
        let texture_path = soil.join(TEXTURE_FILE);
        if !texture_path.is_file() {
            bail!("{} is missing", texture_path.display());
        }
        let texture = point_i32(&texture_path, TEXTURE_VAR, lon, lat)
            .ok()
            .filter(|t| (1..=12).contains(t))
            .unwrap_or(TEXTURE_FILL);
        rows.push((name.clone(), lon, lat, profiles, texture));
    }

    emit(&rows);
    Ok(())
}

/// `<Sitedata>/*_site_v1.nc`，按站点名排序。
fn collect_sites(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        let Some(stem) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(name) = stem.strip_suffix("_site_v1.nc") {
            out.push((name.to_string(), path.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// 城市站点文件的经纬度是 `float longitude(y, x)`，取第一个像元。
fn read_lonlat(file: &Path) -> Result<(f64, f64)> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let scalar = |n: &str| -> Result<f64> {
        let v = f
            .variable(n)
            .with_context(|| format!("{n} not in {}", file.display()))?;
        let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        x.first()
            .copied()
            .with_context(|| format!("{n} is empty in {}", file.display()))
    };
    Ok((scalar("longitude")?, scalar("latitude")?))
}

type Row = (String, f64, f64, Vec<[f64; 8]>, i32);

fn emit(rows: &[Row]) {
    print!("{}", HEADER);

    println!("/// 一个城市站点的土壤剖面。");
    println!("///");
    println!("/// 8 层不是 `nl_soil`（那是 10）—— `mkinidata/MOD_SoilParametersReadin.F90`");
    println!("/// 是 `DO nsl = 1, 8`，栅格每层一个变量也正好 8 个。多写的层 CoLM 不会看。");
    println!("pub struct UrbanSoil {{");
    println!("    pub site: &'static str,");
    println!("    pub lon: f64,");
    println!("    pub lat: f64,");
    for (field, _, _, _) in PROFILE {
        println!("    pub {field}: [f64; 8],");
    }
    println!("    /// USDA 12 类质地，`-1` 表示栅格在这个像元上没有数据。");
    println!("    /// CoLM 把负值夹到 0 再取 `BVIC_USDA(0) = 1.0`，所以 `-1` 要照写。");
    println!("    pub texture: i32,");
    println!("}}");
    println!();

    println!("/// Rust 字段名 → site.nc 变量名。");
    println!("///");
    println!("/// **这张对照表是必须的**：`k_s.nc` 对 `soil_k_s`，而 `BD_all_s.nc` 对");
    println!("/// `soil_BD_all`；`VGM_alpha.nc` 对 `soil_alpha_vgm`。按规则推会错。");
    println!(
        "pub static SITE_VARS: [(&str, &str); {}] = [",
        PROFILE.len() + 1
    );
    for (field, site_var, _, _) in PROFILE {
        println!("    (\"{field}\", \"{site_var}\"),");
    }
    println!("    (\"texture\", \"soil_texture\"),");
    println!("];");
    println!();

    // 数据表不参与 rustfmt —— 不写这个属性的话，下次重新生成又会让
    // `cargo fmt --check` 变红，而把 4053 个数拆成几千行只会让 diff 没法读。
    println!("// **这张表不参与 rustfmt。** 每行是一个站点一个变量的 8 层值，rustfmt 会把");
    println!("// 超宽的数组拆成 8 行 —— 4053 个数拆完之后，重新抽一次数据的 diff 会从");
    println!("// 「哪几个站点变了」变成几千行噪音。`colm-schema/src/generated.rs` 不需要");
    println!("// 这条是因为它的长行 rustfmt 拆不动，会自己放弃。");
    println!("#[rustfmt::skip]");
    println!("pub static SITES: &[UrbanSoil] = &[");
    for (name, lon, lat, profiles, texture) in rows {
        println!("    UrbanSoil {{");
        println!("        site: {name:?},");
        println!("        lon: {lon:?},");
        println!("        lat: {lat:?},");
        for ((field, _, _, _), layers) in PROFILE.iter().zip(profiles) {
            let vals: Vec<String> = layers.iter().map(|x| format!("{x:?}")).collect();
            println!("        {field}: [{}],", vals.join(", "));
        }
        println!("        texture: {texture},");
        println!("    }},");
    }
    println!("];");
    print!("{}", LOOKUP);
}

const HEADER: &str = r#"//! 21 个 Urban-PLUMBER 站点的土壤剖面点值，从 CoLM 2024 rawdata 抽出。
//!
//! **生成的产物，不要手改。** 重生成：
//! `cargo run -p oracle --bin extract-urban-soil -- <Sitedata> <rawdata> > 本文件`
//!
//! **为什么要它**：城市站点文件 23 个变量全是形态学量，一个土壤剖面量都
//! 没有；而 CoLM 的城市路径缺了它们就只能去开 122 GB 的 `soil/`。
//! 城市算例一次只读一格 —— 把那一格预先抽出来，门槛就从 122 GB 落到几十 KB。
//!
//! **这些值是量出来的，不是假设的**：来源是 CoLM 2024 rawdata 在该站点
//! 经纬度上的格点值。写进 site.nc 时的 `source` 属性要说出这一点。

"#;

const LOOKUP: &str = r#"
/// 按经纬度找这个站点的剖面。
///
/// **按经纬度不按名字** —— 名字在两套数据集里会重（`AU-Preston` 在
/// PLUMBER2 与 Urban-PLUMBER 里各有一个），而经纬度是抽取时用的键。
///
/// 反过来，经纬度也不是单射：`US-Minneapolis1` 与 `US-Minneapolis2` 报的是
/// 同一个坐标，落在同一个 500 m 像元里。两条记录的土壤值因此逐位相同，
/// 取到哪一条都不影响结果。
pub fn lookup(lon: f64, lat: f64) -> Option<&'static UrbanSoil> {
    SITES
        .iter()
        .find(|s| (s.lon - lon).abs() < 1e-3 && (s.lat - lat).abs() < 1e-3)
}

#[cfg(test)]
#[path = "urban_soil_tests.rs"]
mod urban_soil_tests;
"#;
