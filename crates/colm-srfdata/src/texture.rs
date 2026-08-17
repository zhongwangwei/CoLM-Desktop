//! CoLM 的 USDA 12 类质地三角。
//!
//! 移植自 `preprocess/rawdata_soil_solids_fractions.F90` 的
//! `USDA_soil_classes` 与 `pointinpolygon`：三角形上 26 个顶点、12 个多边形，
//! 按 (silt, clay) 百分数做点在多边形内判定。
//!
//! **编号必须与 CoLM 一致**：1=clay … 12=sand。这不是可以自选的约定——
//! `MOD_Initialize.F90:420` 是 `BVIC(ipatch) = BVIC_USDA(soiltext(ipatch))`，
//! 编号错一位，VIC 入渗形状参数就静默换一个值。先前的 Python 脚本用了
//! 相反的一套，把 CN-Cng 的 silty loam(8, BVIC 0.100) 写成了
//! clay loam(4, BVIC 0.230)。
//!
//! 顶点与边上都算「在内」，与 CoLM 一致；同时命中多个类时取编号最大的，
//! 因为 CoLM 的调用方是连续的 `IF(c(k))` 赋值，后匹配覆盖先匹配。

/// 三角形里 26 个顶点的 silt 坐标（百分数）。
const XPOS: [f64; 26] = [
    0.0, 40.0, 0.0, 20.0, 15.0, 40.0, 60.0, 0.0, 27.5, 27.5, 50.0, 52.5, 72.5, 0.0, 0.0, 40.0,
    50.0, 80.0, 87.5, 15.0, 30.0, 50.0, 80.0, 0.0, 0.0, 100.0,
];

/// 同上，clay 坐标。
const YPOS: [f64; 26] = [
    55.0, 60.0, 35.0, 35.0, 40.0, 40.0, 40.0, 20.0, 20.0, 27.5, 27.5, 27.5, 27.5, 15.0, 10.0, 7.5,
    7.5, 12.5, 12.5, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0,
];

/// 12 个多边形，元素是 `XPOS`/`YPOS` 的 1-based 序号。
const POLYGONS: [&[usize]; 12] = [
    &[24, 1, 5, 6, 2],
    &[2, 6, 7],
    &[1, 3, 4, 5],
    &[5, 4, 10, 11, 12, 6],
    &[6, 12, 13, 7],
    &[3, 8, 9, 10, 4],
    &[10, 9, 16, 17, 11],
    &[11, 17, 22, 23, 18, 19, 13, 12],
    &[8, 14, 21, 22, 17, 16, 9],
    &[18, 23, 26, 19],
    &[14, 15, 20, 21],
    &[15, 25, 20],
];

/// 类名，下标 0 对应类别 1。顺序即 CoLM 的编号。
pub const CLASS_NAMES: [&str; 12] = [
    "clay",
    "silty clay",
    "sandy clay",
    "clay loam",
    "silty clay loam",
    "sandy clay loam",
    "loam",
    "silty loam",
    "sandy loam",
    "silt",
    "loamy sand",
    "sand",
];

/// `MOD_Initialize.F90:271` 的 `BVIC_USDA(0:12)`。
/// 下标 0 是 CoLM 对越界质地的兜底值，不是一个真实类别。
pub const BVIC_USDA: [f64; 13] = [
    1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180, 0.100, 0.090, 0.150, 0.080, 0.050,
];

/// 按 silt / clay 百分数定质地类别，返回 1..=12。
///
/// 落在三角形外返回 `None`——不猜。CoLM 会把越界值静默置 0，
/// 而 `BVIC_USDA(0) = 1.0`，那是个比任何正常类别都大得多的入渗参数。
pub fn classify(silt: f64, clay: f64) -> Option<u8> {
    if !silt.is_finite() || !clay.is_finite() {
        return None;
    }
    if silt < 0.0 || clay < 0.0 || silt + clay > 100.0 + 1e-9 {
        return None;
    }
    let mut hit = None;
    for (k, poly) in POLYGONS.iter().enumerate() {
        let xs: Vec<f64> = poly.iter().map(|p| XPOS[p - 1]).collect();
        let ys: Vec<f64> = poly.iter().map(|p| YPOS[p - 1]).collect();
        if point_in_polygon(silt, clay, &xs, &ys) {
            hit = Some((k + 1) as u8); // 后匹配覆盖先匹配，与 CoLM 一致
        }
    }
    hit
}

/// 顶点、边上、内部都算在内，与 CoLM 的 `pointinpolygon` 一致。
fn point_in_polygon(xp: f64, yp: f64, xpol: &[f64], ypol: &[f64]) -> bool {
    let n = xpol.len();
    let (mut rcross, mut lcross) = (0usize, 0usize);
    for i in 0..n {
        if xpol[i] - xp == 0.0 && ypol[i] - yp == 0.0 {
            return true; // 顶点
        }
        let i1 = (i + n - 1) % n;
        if ((ypol[i] - yp) > 0.0) != ((ypol[i1] - yp) > 0.0) {
            let x = ((xpol[i] - xp) * (ypol[i1] - yp) - (xpol[i1] - xp) * (ypol[i] - yp))
                / (ypol[i1] - ypol[i]);
            if x > 0.0 {
                rcross += 1;
            }
        }
        if ((ypol[i] - yp) < 0.0) != ((ypol[i1] - yp) < 0.0) {
            let x = ((xpol[i] - xp) * (ypol[i1] - yp) - (xpol[i1] - xp) * (ypol[i] - yp))
                / (ypol[i1] - ypol[i]);
            if x < 0.0 {
                lcross += 1;
            }
        }
    }
    if rcross % 2 != lcross % 2 {
        return true; // 边上
    }
    rcross % 2 == 1
}

#[cfg(test)]
#[path = "texture_tests.rs"]
mod texture_tests;
