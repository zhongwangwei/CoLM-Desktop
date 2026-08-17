//! 由站点文件已有量推导 CoLM 还要的三个土壤字段，以及 0–60 cm 深度加权。
//!
//! **三种量用了三套基准**，混用会算出负的剩余量。见
//! `preprocess/rawdata_soil_solids_fractions.F90` 与
//! `preprocess/rd_soil_properties.F90:504`：
//!
//! - `wf_om` 与 `wf_gravels` 是**全土**质量分数；
//! - `wf_sand` 入库前被 `wf_sand_s = soil_sand_l / 100.0` 覆盖过，是**细土**分数；
//! - `vf_sand` / `vf_gravels` / `vf_om` 是**固体内**体积分数。
//!
//! 实测 US-NR1 的 `wf_sand = 0.82` 与 `wf_gravels = 0.5488` 并存，两者相加
//! 已超过 1。把它们当同一套去减，17/90 个站点会算出负的粉粒分数。
//!
//! 于是：`wf_om = OM_density / BD_all`（CoLM 自己的恒等式，
//! `OM_density = BD_ave * wf_om_s * 1000`）、`wf_clay` 取 `wf_sand` 的剩余量、
//! `vf_clay` 取三个体积分数的剩余量。
//!
//! 剩余量按 1:3 的黏:粉劈开是一个**假设**：站点文件不给黏粒，而 CoLM 无条件
//! 要它。这个假设必须显式写进产物的 `source` 属性里 —— 用户有权知道哪些数是
//! 量出来的、哪些是猜的。
//!
//! 深度加权用于质地分类：CoLM 的回落栅格是
//! `soil/soiltexture_0cm-60cm_mean.nc`，即 0–60 cm 的平均，所以这里也取 0–60 cm。
//!
//! 层数：实测 PLUMBER2 站点文件的土壤剖面是 **10 层**，而 `MOD_SingleSrfdata`
//! 只用前 8 层。推导出的数组与源数组等长（10），深度加权只覆盖前 8 层。

/// CoLM 标准 10 层土壤厚度（m），srfdata 只用前 8 层。
pub const DZ_SOIL: [f64; 8] = [
    0.0175, 0.0276, 0.0455, 0.0750, 0.1236, 0.2038, 0.3360, 0.5539,
];

/// 站点文件里已有的土壤剖面量。注意三组量的基准并不相同，见模块文档。
#[derive(Debug, Clone)]
pub struct SoilColumn {
    /// 固体内体积分数
    pub vf_sand: Vec<f64>,
    /// 固体内体积分数
    pub vf_gravels: Vec<f64>,
    /// 固体内体积分数
    pub vf_om: Vec<f64>,
    /// **细土**质量分数
    pub wf_sand: Vec<f64>,
    /// kg/m^3
    pub om_density: Vec<f64>,
    /// kg/m^3
    pub bd_all: Vec<f64>,
}

/// 推导出来的三个字段。
#[derive(Debug, Clone)]
pub struct Derived {
    pub vf_clay: Vec<f64>,
    pub wf_clay: Vec<f64>,
    pub wf_om: Vec<f64>,
}

/// 细土的砂/粉/黏百分数，喂给质地分类器。
#[derive(Debug, Clone, Copy)]
pub struct FineEarth {
    pub sand: f64,
    pub silt: f64,
    pub clay: f64,
}

/// 各层落在 `0..depth` 以内的厚度（m）。深度以下的层权重为 0。
pub fn depth_weights(depth: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(DZ_SOIL.len());
    let mut top = 0.0;
    for dz in DZ_SOIL {
        let bot = top + dz;
        out.push((bot.min(depth) - top.min(depth)).max(0.0));
        top = bot;
    }
    out
}

/// 推导 `vf_clay` / `wf_clay` / `wf_om`。三者各自用自己基准里的剩余量。
pub fn derive(c: &SoilColumn) -> Derived {
    let n = c.wf_sand.len();
    let wf_om = (0..n)
        .map(|i| {
            // BD_all 为 0 时不做除法：inf 写进文件之后会一路走到能量平衡里。
            if c.bd_all[i] > 0.0 {
                (c.om_density[i] / c.bd_all[i]).clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .collect();
    let wf_clay = (0..n)
        .map(|i| 0.25 * (1.0 - c.wf_sand[i]).clamp(0.0, 1.0))
        .collect();
    let vf_clay = (0..n)
        .map(|i| 0.25 * (1.0 - c.vf_sand[i] - c.vf_gravels[i] - c.vf_om[i]).clamp(0.0, 1.0))
        .collect();
    Derived {
        vf_clay,
        wf_clay,
        wf_om,
    }
}

/// 0–60 cm 深度加权的细土砂/粉/黏百分数。
///
/// `wf_sand` 已经是细土分数，所以这里**不减**砾石与有机质 —— 它们是别的基准。
pub fn fine_earth_fractions(c: &SoilColumn) -> FineEarth {
    let w = depth_weights(0.60);
    let mut sand = 0.0;
    let mut wsum = 0.0;
    // 只走到剖面真正有的层数：实测 PLUMBER2 站点文件是 10 层，CoLM 只用前 8 层，
    // 但层数更少的文件不该以数组越界收场。
    for (i, &wi) in w.iter().enumerate().take(c.wf_sand.len()) {
        if wi <= 0.0 {
            continue;
        }
        sand += wi * c.wf_sand[i];
        wsum += wi;
    }
    if wsum <= 0.0 {
        return FineEarth {
            sand: 0.0,
            silt: 0.0,
            clay: 0.0,
        };
    }
    let sand = (100.0 * sand / wsum).clamp(0.0, 100.0);
    let rest = 100.0 - sand;
    FineEarth {
        sand,
        silt: 0.75 * rest,
        clay: 0.25 * rest,
    }
}

#[cfg(test)]
#[path = "derive_tests.rs"]
mod derive_tests;
