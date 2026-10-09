//! 网格河湖的堤防（`DEF_USE_LEVEE`，`MOD_Grid_RiverLakeLevee`）。
//!
//! 每个单元流域可以有一道离河槽 `levee_dst` 远、高 `levee_hgt` 的堤：漫滩水先在堤外（河槽一侧）
//! 涨，漫过堤顶后才进到堤内的受保护区，那部分蓄量是 `levsto`、水深是 `levdph`。
//! 这里移植初始化（`levee_init`）与全部水位—库容关系；汇流、续跑、history 的接入在调用方。
//!
//! 数值形状照 latlon 内核的 GIMPLE（模块里只有 20 个 FMA，逐个对过）：
//! - 初始化：`fldstomax(j) = FMA(FMA(j-0.5, dwth_inc, rivwth)*dhgt, rivlen, fil)`、
//!   `bassto = FMA((FMA(add, 0.5, fil) + rivwth)*(add*fldgrd), rivlen, fil)`、
//!   `topsto = FMA((dst + rivwth)*dhgtdif, rivlen, bassto)`；
//! - 抛物线求宽：`sqrt(max(FMA(w, w, ((2*dsto)/rivlen)/fldgrd), 0)) - w`；
//! - 全剖面的逐层宽度：`FMA(dwth_inc, j, rivwth)`；堤内那段：`FMS(j, dwth_inc, dst)`。

// 夹紧保留上游 `MIN(MAX(·))` 的次序；逐层下标与上游的层号循环一一对应。
#![allow(clippy::manual_clamp, clippy::needless_range_loop)]

use colm_numeric::Contract;
use std::path::Path;

use anyhow::{ensure, Context, Result};

use super::network::RiverNetwork;

/// 每个单元流域的堤防几何（`levee_init` 的结果）。下标与网络一致；逐层量按 `j = 1..=nlfp`
/// 存在 `[j-1]`。
#[derive(Debug, Clone)]
pub struct Levee {
    pub has: Vec<bool>,
    pub frc: Vec<f64>,
    pub hgt: Vec<f64>,
    pub dst: Vec<f64>,
    pub bashgt: Vec<f64>,
    pub bassto: Vec<f64>,
    pub topsto: Vec<f64>,
    pub filsto: Vec<f64>,
    pub fldgrd: Vec<Vec<f64>>,
    pub fldstomax: Vec<Vec<f64>>,
}

/// 一个水位分区的结果（`levee_fldstg` 的四个出参）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeveeStage {
    /// 河槽与堤外的水深（m）。
    pub wdsrf: f64,
    /// 堤内受保护的蓄量（m³）。
    pub levsto: f64,
    /// 堤内水深（m）。
    pub levdph: f64,
    /// 淹没比例。
    pub fldfrc: f64,
}

fn read_vector(file: &netcdf::File, name: &str, n: usize, path: &Path) -> Result<Vec<f64>> {
    let values = file
        .variable(name)
        .with_context(|| format!("{} has no {name}; DEF_USE_LEVEE needs it", path.display()))?
        .get_values::<f64, _>(..)?;
    ensure!(
        values.len() == n,
        "{name} in {} has {} values for {n} unit catchments",
        path.display(),
        values.len()
    );
    Ok(values)
}

impl Levee {
    /// 读 `levee_frc`/`levee_hgt` 并做 `levee_init`（不接水库：`lake_type == 2` 的冲突处理不会出现）。
    ///
    /// `reservoir_cells[i]` 为真表示单元流域 `i` 在水库表里（`lake_type == 2`，不论是否已建成）：
    /// 上游在建几何之前把这些单元流域强制设成无堤。
    pub fn read(path: &Path, network: &RiverNetwork, reservoir_cells: &[bool]) -> Result<Self> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let n = network.len();
        let mut frc = read_vector(&file, "levee_frc", n, path)?;
        let mut hgt = read_vector(&file, "levee_hgt", n, path)?;
        // `levee_init` 先做输入校验再处理水库冲突；冲突的单元流域按无效输入同样处理
        // （`frc = 1`、`hgt = 0`），结果与上游先校验后覆盖相同。
        for (i, &reservoir) in reservoir_cells.iter().enumerate() {
            if reservoir {
                frc[i] = 1.0;
                hgt[i] = 0.0;
            }
        }
        Ok(Self::new(network, frc, hgt))
    }

    /// `levee_init`。
    pub fn new(network: &RiverNetwork, mut frc: Vec<f64>, mut hgt: Vec<f64>) -> Self {
        let n = network.len();
        for i in 0..n {
            let invalid = !hgt[i].is_finite()
                || !frc[i].is_finite()
                || hgt[i] <= 0.0
                || frc[i] >= 1.0
                || frc[i] < 0.0;
            if invalid {
                hgt[i] = 0.0;
                frc[i] = 1.0;
            }
            frc[i] = frc[i].min(1.0).max(0.0);
        }
        let mut has: Vec<bool> = frc.iter().map(|&f| f < 1.0).collect();
        let mut fldgrd = vec![Vec::new(); n];
        let mut fldstomax = vec![Vec::new(); n];
        for i in 0..n {
            let curve = &network.curves[i];
            let nlfp = curve.flphgt.len() - 1;
            fldgrd[i] = vec![0.0; nlfp];
            fldstomax[i] = vec![0.0; nlfp];
            let (rivlen, rivwth, area) = (network.rivlen[i], network.rivwth[i], network.area[i]);
            if rivlen <= 0.0 || nlfp == 0 || area <= 0.0 {
                continue;
            }
            let dwth_inc = area / (rivlen * nlfp as f64);
            let mut dsto_fil = curve.rivstomax;
            let mut dhgtpre = 0.0;
            for j in 1..=nlfp {
                let mut dhgtnow = curve.flphgt[j] - dhgtpre;
                if dhgtnow > 0.0 && dwth_inc > 0.0 {
                    fldgrd[i][j - 1] = dhgtnow / dwth_inc;
                } else {
                    fldgrd[i][j - 1] = 0.0;
                    dhgtnow = 0.0;
                }
                let width = (j as f64 - 0.5).contract(dwth_inc, rivwth);
                fldstomax[i][j - 1] = (width * dhgtnow).contract(rivlen, dsto_fil);
                dsto_fil = fldstomax[i][j - 1];
                dhgtpre = curve.flphgt[j];
            }
        }
        let mut levee = Self {
            has: Vec::new(),
            frc,
            hgt,
            dst: vec![0.0; n],
            bashgt: vec![0.0; n],
            bassto: vec![0.0; n],
            topsto: vec![0.0; n],
            filsto: vec![0.0; n],
            fldgrd,
            fldstomax,
        };
        for i in 0..n {
            if !has[i] {
                continue;
            }
            let curve = &network.curves[i];
            let nlfp = curve.flphgt.len() - 1;
            let (rivlen, rivwth, area) = (network.rivlen[i], network.rivwth[i], network.area[i]);
            if rivlen <= 0.0 || nlfp == 0 || area <= 0.0 {
                has[i] = false;
                continue;
            }
            levee.dst[i] = levee.frc[i] * (area / rivlen);
            let dwth_inc = area / (rivlen * nlfp as f64);
            let ilev = ((levee.frc[i] * nlfp as f64) as i64 + 1)
                .max(1)
                .min(nlfp as i64) as usize;
            let (dhgtpre, dwth_fil) = if ilev >= 2 {
                (curve.flphgt[ilev - 1], dwth_inc * (ilev - 1) as f64)
            } else {
                (0.0, 0.0)
            };
            let dwth_add = (levee.dst[i] - dwth_fil).max(0.0);
            let grade = levee.fldgrd[i][ilev - 1];
            let rise = dwth_add * grade;
            levee.bashgt[i] = dhgtpre + rise;
            levee.hgt[i] = levee.hgt[i].max(levee.bashgt[i]);
            let dsto_fil = if ilev >= 2 {
                levee.fldstomax[i][ilev - 2]
            } else {
                curve.rivstomax
            };
            levee.bassto[i] = dsto_fil;
            if dwth_add > 0.0 && grade > 0.0 {
                let width = dwth_add.contract(0.5, dwth_fil) + rivwth;
                levee.bassto[i] = (width * rise).contract(rivlen, dsto_fil);
            }
            let dhgtdif = levee.hgt[i] - levee.bashgt[i];
            levee.topsto[i] = ((levee.dst[i] + rivwth) * dhgtdif).contract(rivlen, levee.bassto[i]);
            levee.filsto[i] =
                levee.total_volume_from_depth(network, i, curve.rivhgt + levee.hgt[i]);
            levee.topsto[i] = levee.topsto[i].max(levee.bassto[i]);
            levee.filsto[i] = levee.filsto[i].max(levee.topsto[i]);
        }
        levee.has = has;
        levee
    }

    /// `levee_total_volume_from_depth`：不分堤内外的全剖面库容。
    pub fn total_volume_from_depth(&self, network: &RiverNetwork, i: usize, wdsrf: f64) -> f64 {
        let curve = &network.curves[i];
        if wdsrf <= 0.0 {
            return 0.0;
        }
        if wdsrf <= curve.rivhgt {
            return wdsrf * curve.rivare;
        }
        let (rivlen, rivwth, area) = (network.rivlen[i], network.rivwth[i], network.area[i]);
        let nlfp = curve.flphgt.len() - 1;
        if rivlen <= 0.0 || area <= 0.0 || nlfp == 0 {
            return curve.volume(wdsrf);
        }
        let flddph = wdsrf - curve.rivhgt;
        let dwth_inc = area / (rivlen * nlfp as f64);
        let mut j = 1;
        let mut dsto_fil = curve.rivstomax;
        let mut dwth_fil = rivwth;
        let mut dhgtpre = 0.0;
        while j <= nlfp {
            if flddph <= curve.flphgt[j] {
                break;
            }
            dsto_fil = self.fldstomax[i][j - 1];
            dwth_fil = dwth_inc.contract(j as f64, rivwth);
            dhgtpre = curve.flphgt[j];
            j += 1;
        }
        let dhgtnow = (flddph - dhgtpre).max(0.0);
        let dsto_add = if j <= nlfp {
            let grade = self.fldgrd[i][j - 1];
            let half = if grade > 0.0 {
                dhgtnow / grade * 0.5
            } else {
                0.0
            };
            rivlen * (dhgtnow * (dwth_fil + half))
        } else {
            rivlen * (dhgtnow * dwth_fil)
        };
        (dsto_add + dsto_fil).max(0.0)
    }

    /// `levee_total_depth`：全剖面库容对应的水深。
    pub fn total_depth(&self, network: &RiverNetwork, i: usize, volume: f64) -> f64 {
        let curve = &network.curves[i];
        if volume <= 0.0 {
            return 0.0;
        }
        if volume <= curve.rivstomax {
            return if curve.rivare > 0.0 {
                volume / curve.rivare
            } else {
                0.0
            };
        }
        let (rivlen, rivwth, area) = (network.rivlen[i], network.rivwth[i], network.area[i]);
        let nlfp = curve.flphgt.len() - 1;
        if rivlen <= 0.0 || area <= 0.0 || nlfp == 0 {
            return curve.depth(volume);
        }
        let dwth_inc = area / (rivlen * nlfp as f64);
        let mut j = 1;
        let mut dsto_fil = curve.rivstomax;
        let mut dwth_fil = rivwth;
        let mut ddph_fil = 0.0;
        while j <= nlfp {
            if volume <= self.fldstomax[i][j - 1] {
                break;
            }
            dsto_fil = self.fldstomax[i][j - 1];
            dwth_fil = dwth_inc.contract(j as f64, rivwth);
            ddph_fil = curve.flphgt[j];
            j += 1;
        }
        let dsto_add = (volume - dsto_fil).max(0.0);
        let base = ddph_fil + curve.rivhgt;
        if j <= nlfp {
            let grade = self.fldgrd[i][j - 1];
            if grade > 0.0 {
                let dwth_add = widen(dwth_fil, dsto_add, rivlen, grade);
                grade.contract(dwth_add, base)
            } else {
                base
            }
        } else if dwth_fil > 0.0 && rivlen > 0.0 {
            dsto_add / dwth_fil / rivlen + base
        } else {
            base
        }
    }

    /// `levee_total_flood_fraction_from_depth`。
    pub fn flood_fraction_from_depth(&self, network: &RiverNetwork, i: usize, wdsrf: f64) -> f64 {
        let curve = &network.curves[i];
        if wdsrf <= curve.rivhgt {
            return 0.0;
        }
        let (rivlen, area) = (network.rivlen[i], network.area[i]);
        let nlfp = curve.flphgt.len() - 1;
        if rivlen <= 0.0 || area <= 0.0 || nlfp == 0 {
            let fldfrc = if area > 0.0 {
                curve.floodarea(wdsrf) / area
            } else {
                0.0
            };
            return fldfrc.max(0.0).min(1.0);
        }
        let flddph = wdsrf - curve.rivhgt;
        let rivwth = network.rivwth[i].max(0.0);
        let dwth_inc = area / (rivlen * nlfp as f64);
        let mut j = 1;
        let mut dwth_fil = rivwth;
        let mut dhgtpre = 0.0;
        while j <= nlfp {
            if flddph <= curve.flphgt[j] {
                break;
            }
            dwth_fil = dwth_inc.contract(j as f64, rivwth);
            dhgtpre = curve.flphgt[j];
            j += 1;
        }
        let dwth_add = if j <= nlfp {
            let grade = self.fldgrd[i][j - 1];
            if grade > 0.0 {
                (flddph - dhgtpre).max(0.0) / grade
            } else {
                0.0
            }
        } else {
            0.0
        };
        let fldfrc = if dwth_inc > 0.0 {
            (dwth_fil - rivwth + dwth_add) / (nlfp as f64 * dwth_inc)
        } else {
            0.0
        };
        fldfrc.max(0.0).min(1.0)
    }

    /// `levee_fldstg`：总库容分到河槽/堤外与堤内。
    pub fn fldstg(&self, network: &RiverNetwork, i: usize, vol_total: f64) -> LeveeStage {
        let curve = &network.curves[i];
        let rivstomax = curve.rivstomax;
        let area = network.area[i];
        if !self.has[i] {
            let wdsrf = curve.depth(vol_total);
            let fldfrc = if area > 0.0 {
                curve.floodarea(wdsrf) / area
            } else {
                0.0
            };
            return LeveeStage {
                wdsrf,
                levsto: 0.0,
                levdph: 0.0,
                fldfrc,
            };
        }
        let (rivlen, rivwth) = (network.rivlen[i], network.rivwth[i]);
        let nlfp = curve.flphgt.len() - 1;
        let nlfp_f = nlfp as f64;
        let dwth_inc = if rivlen > 0.0 && nlfp > 0 {
            area / (nlfp_f * rivlen)
        } else {
            0.0
        };
        let frc = self.frc[i];
        let dst = self.dst[i];
        let bashgt = self.bashgt[i];
        let hgt = self.hgt[i];
        if vol_total <= rivstomax {
            return LeveeStage {
                wdsrf: if curve.rivare > 0.0 {
                    vol_total / curve.rivare
                } else {
                    0.0
                },
                levsto: 0.0,
                levdph: 0.0,
                fldfrc: 0.0,
            };
        }
        if vol_total < self.bassto[i] {
            // 堤外还没淹到堤脚：逐层累加宽度（`dwth_fil + dwth_inc`，不是 `FMA(dwth_inc, j, ·)`）。
            let mut j = 1;
            let mut dsto_fil = rivstomax;
            let mut dwth_fil = rivwth;
            let mut ddph_fil = 0.0;
            while j <= nlfp {
                if vol_total <= self.fldstomax[i][j - 1] {
                    break;
                }
                dsto_fil = self.fldstomax[i][j - 1];
                dwth_fil += dwth_inc;
                ddph_fil = curve.flphgt[j];
                j += 1;
            }
            let dsto_add = vol_total - dsto_fil;
            let (dwth_add, flddph) = if j <= nlfp {
                let grade = self.fldgrd[i][j - 1];
                let add = if grade > 0.0 {
                    widen(dwth_fil, dsto_add, rivlen, grade)
                } else {
                    0.0
                };
                (add, add.contract(grade, ddph_fil))
            } else if dwth_fil > 0.0 {
                (0.0, dsto_add / dwth_fil / rivlen + ddph_fil)
            } else {
                (0.0, ddph_fil)
            };
            let fldfrc = (dwth_fil - rivwth + dwth_add) / (nlfp_f * dwth_inc);
            return LeveeStage {
                wdsrf: curve.rivhgt + flddph,
                levsto: 0.0,
                levdph: 0.0,
                fldfrc: fldfrc.max(0.0).min(frc),
            };
        }
        if vol_total < self.topsto[i] {
            // 堤外淹到堤脚之后、漫顶之前：水面在堤与河槽之间等宽地抬升。
            let width = (dst + rivwth) * rivlen;
            let flddph = if width > 0.0 {
                (vol_total - self.bassto[i]) / width + bashgt
            } else {
                bashgt
            };
            return LeveeStage {
                wdsrf: curve.rivhgt + flddph,
                levsto: 0.0,
                levdph: 0.0,
                fldfrc: frc,
            };
        }
        if vol_total < self.filsto[i] {
            // 漫过堤顶、堤内还没灌满：堤外水位停在堤顶，多出来的进堤内。
            let rivsto = hgt.contract(rivlen * rivwth, rivstomax);
            let unprotected = (self.topsto[i] - rivsto).max(0.0);
            let levsto = (vol_total - rivsto - unprotected).max(0.0);
            let ilev = ((frc * nlfp_f) as i64 + 1).max(1).min(nlfp as i64) as usize;
            let mut dsto_fil = self.topsto[i];
            let mut dwth_fil = 0.0;
            let mut ddph_fil = 0.0;
            let mut j = ilev;
            while j <= nlfp {
                let level = ((dst + rivwth) * (hgt - curve.flphgt[j]))
                    .contract(rivlen, self.fldstomax[i][j - 1]);
                if vol_total < level {
                    break;
                }
                dsto_fil = level;
                dwth_fil = (j as f64).contract(dwth_inc, -dst);
                ddph_fil = curve.flphgt[j] - bashgt;
                j += 1;
            }
            let dsto_add = vol_total - dsto_fil;
            let (levdph, fldfrc) = if j <= nlfp {
                let grade = self.fldgrd[i][j - 1];
                let add = if grade > 0.0 {
                    widen(dwth_fil, dsto_add, rivlen, grade)
                } else {
                    0.0
                };
                let fraction = (add + dwth_fil + dst) / (nlfp_f * dwth_inc);
                (
                    add.contract(grade, bashgt + ddph_fil),
                    fraction.max(0.0).min(1.0),
                )
            } else {
                let add = if dwth_fil > 0.0 {
                    dsto_add / dwth_fil / rivlen
                } else {
                    0.0
                };
                (bashgt + ddph_fil + add, 1.0)
            };
            return LeveeStage {
                wdsrf: curve.rivhgt + hgt,
                levsto,
                levdph,
                fldfrc,
            };
        }
        // 堤内灌满：内外连通，按全剖面算水深，再把堤内那份分出来。
        let wdsrf = self.total_depth(network, i, vol_total);
        let flddph = wdsrf - curve.rivhgt;
        let fldfrc = self.flood_fraction_from_depth(network, i, wdsrf);
        let rivsto = (rivlen * rivwth).contract(flddph, rivstomax);
        let unprotected =
            (((flddph - hgt) * (dst + rivwth)).contract(rivlen, self.topsto[i]) - rivsto).max(0.0);
        LeveeStage {
            wdsrf,
            levsto: (vol_total - rivsto - unprotected).max(0.0),
            levdph: flddph,
            fldfrc,
        }
    }

    /// `levee_visible_volume_from_stage`：已知水位与堤内蓄量时堤外（可见）的库容。
    pub fn visible_volume_from_stage(
        &self,
        network: &RiverNetwork,
        i: usize,
        wdsrf: f64,
        levsto: f64,
    ) -> f64 {
        let curve = &network.curves[i];
        if !self.has[i] {
            return curve.volume(wdsrf);
        }
        let flddph = wdsrf - curve.rivhgt;
        let volume = if flddph <= self.bashgt[i] {
            self.total_volume_from_depth(network, i, wdsrf)
        } else if flddph <= self.hgt[i] + 1.0e-6 {
            ((self.dst[i] + network.rivwth[i]) * (flddph - self.bashgt[i]).max(0.0))
                .contract(network.rivlen[i], self.bassto[i])
        } else {
            self.total_volume_from_depth(network, i, wdsrf) - levsto.max(0.0)
        };
        volume.max(0.0)
    }
}

/// 抛物线剖面上加宽多少才装得下 `dsto_add`：`sqrt(max(w² + 2*dsto/rivlen/grade, 0)) - w`。
fn widen(dwth_fil: f64, dsto_add: f64, rivlen: f64, grade: f64) -> f64 {
    dwth_fil
        .contract(dwth_fil, dsto_add * 2.0 / rivlen / grade)
        .max(0.0)
        .sqrt()
        - dwth_fil
}

#[cfg(test)]
#[path = "levee_tests.rs"]
mod levee_tests;
