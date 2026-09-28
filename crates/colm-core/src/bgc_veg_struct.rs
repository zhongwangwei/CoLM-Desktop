//! `MOD_BGC_Veg_CNVegStructUpdate.F90`：由 C 池更新 LAI/SAI。
//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 按上游结构保留，便于逐行对照。
#![allow(clippy::collapsible_if, clippy::collapsible_else_if)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};
use crate::bgc_state::BgcState;

/// `CNVegStructUpdate`：更新 `tsai_p`（每步）与 LAI 反馈下的 `tlai_p`/`lai_p`，再汇总 patch LAI。
pub fn cn_veg_struct_update(
    s: &mut BgcState,
    p: &mut BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let npft = p.pftclass.len();
    let mut tlai_old: f64;
    let mut tsai_old: f64;
    let mut tsai_min: f64;
    let mut tsai_alpha: f64;
    let dtsmonth = 2592000.0;
    let natlaimx = 8.0;
    let theta = 0.8;
    p.lai[0] = 0.0;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if ivt != 0 {
            tlai_old = p.tlai_p[m];
            tsai_old = p.tsai_p[m];
            if sw.laifeedback {
                p.tlai_p[m] = ((c.slatop[class].mul_add(s.pft.leafc_p[m], natlaimx))
                    - ((c.slatop[class].mul_add(s.pft.leafc_p[m], natlaimx)).mul_add(
                        c.slatop[class].mul_add(s.pft.leafc_p[m], natlaimx),
                        -(4.0 * theta * natlaimx * c.slatop[class] * s.pft.leafc_p[m]),
                    ))
                    .sqrt())
                    / (2.0 * theta);
                p.tlai_p[m] = 0.0_f64.max(p.tlai_p[m]);
                p.lai_p[m] = p.tlai_p[m];
            }
            if ivt == 15 || ivt == 16 {
                tsai_alpha = 1.0 - (1.0 * p.deltim / dtsmonth);
                tsai_min = 0.1;
            } else {
                tsai_alpha = 1.0 - (0.5 * p.deltim / dtsmonth);
                tsai_min = 1.0;
            }
            tsai_min *= 0.5;
            p.tsai_p[m] =
                (tsai_alpha.mul_add(tsai_old, (tlai_old - p.tlai_p[m]).max(0.0))).max(tsai_min);
            if c.woody[class] == 1.0 {
            } else if ivt >= NPCROPMIN {
                if sw.crop {
                    if p.tlai_p[m] >= c.laimx[class] {
                        s.pft.peaklai_p[m] = 1;
                    }
                    if ivt == 17
                        || ivt == 18
                        || ivt == 75
                        || ivt == 76
                        || ivt == 67
                        || ivt == 68
                        || ivt == 71
                        || ivt == 72
                        || ivt == 73
                        || ivt == 74
                    {
                        p.tsai_p[m] = 0.1 * p.tlai_p[m];
                    } else {
                        p.tsai_p[m] = 0.2 * p.tlai_p[m];
                    }
                    if s.pft.harvdate_p[m] < 999.0 && p.tlai_p[m] == 0.0 {
                        s.pft.peaklai_p[m] = 0;
                        if sw.fire {
                            p.tsai_p[m] = 0.25 * ((-s.patch.farea_burned[0]).mul_add(0.90, 1.0));
                        }
                    }
                }
            }
        }
        p.lai[0] = p.lai_p[m].mul_add(p.pftfrac[m], p.lai[0]);
    }
    p.tlai[0] = p.lai[0];
}
