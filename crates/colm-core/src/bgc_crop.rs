//! `#ifdef CROP` 下转写器处理不了的作物过程（其余作物支由 `oracle/scripts/bgc_port/regen.py` 生成）。
//!
//! `vernalization`（`MOD_BGC_Veg_CNPhenology.F90:1577-1611`）：`vtmin/vtopt/vtmax` 是局部常数，gfortran -O2
//! 把 `alpha = log(2)/log((vtmax-vtmin)/(vtopt-vtmin))`、`(vtopt-vtmin)**alpha`、`(vtopt-vtmin)**(2*alpha)`、
//! `2*alpha` 与 `22.5**5` 全部在编译期（MPFR，正确舍入）折成常数，只有依赖 `tc` 的两次幂与 `cumvd**5`
//! 在运行期调 libm `pow`。运行期再算一遍 `ln` 不保证与 MPFR 逐位相同，所以这里直接写 GIMPLE 里的常数。

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;
use crate::LibmPow;

/// `alpha`（编译期折叠）。
const ALPHA: f64 = 0.687_193_302_053_350_5;
/// `2*alpha`（编译期折叠）。
const TWO_ALPHA: f64 = 1.374_386_604_106_701;
/// `(vtopt - vtmin)**alpha`（编译期折叠）。
const OPT_POW_ALPHA: f64 = 3.503_694_743_729_268_3;
/// `(vtopt - vtmin)**(2*alpha)`（编译期折叠）。
const OPT_POW_TWO_ALPHA: f64 = 12.275_876_857_236_103;
/// `22.5**5`。
const VF_HALF_POW_5: f64 = 5_766_503.906_25;

/// `vernalization(i, m, deltim)`：冬小麦春化，`m` 是 1 起的 PFT 下标（与生成代码的调用约定一致）。
pub fn vernalization(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    _sw: BgcSwitches,
    m: i32,
) {
    let m = (m - 1) as usize;
    let dt = p.deltim / 3600.0;
    let tc = p.tref_p[m] - 273.16;
    if (-1.3..=15.7).contains(&tc) {
        // GIMPLE：`x = tc + 1.3`；`FMS (2*pow(x,α), C1, pow(x,2α)) / C2`，再 `FMA (…, dt/24, cumvd)`。
        let x = tc + 1.3;
        let rate =
            (x.lpow(ALPHA) * 2.0).mul_add(OPT_POW_ALPHA, -x.lpow(TWO_ALPHA)) / OPT_POW_TWO_ALPHA;
        s.pft.cumvd_p[m] = rate.mul_add(dt / 24.0, s.pft.cumvd_p[m]);
    }
    let cumvd_pow_5 = s.pft.cumvd_p[m].lpow(5.0);
    s.pft.vf_p[m] = cumvd_pow_5 / (cumvd_pow_5 + VF_HALF_POW_5);
}

#[cfg(test)]
#[path = "bgc_crop_tests.rs"]
mod bgc_crop_tests;
