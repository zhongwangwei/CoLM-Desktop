//! `MOD_BGC_Veg_CNNDynamics.F90` 的作物部分：施肥与大豆固氮（`#ifdef CROP`）。
//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 与"先声明、分支里赋值"都按上游结构保留，便于逐行对照。
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::needless_late_init
)]
// `a >= lo .and. a <= hi`、`max(lo, min(hi, x))` 照抄：改成 `contains`/`clamp` 会改变 NaN 的行为。
#![allow(clippy::manual_range_contains, clippy::manual_clamp)]
// `x = x * y` 照上游一条赋值写（与 `*=` 舍入相同，保留以便对照）。
#![allow(clippy::assign_op_pattern)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;
use crate::LibmPow;
use colm_numeric::Contract;

/// `CNNFert`：施肥进入土壤矿质 N。
pub fn cn_n_fert(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, _sw: BgcSwitches) {
    let npft = p.pftclass.len();
    s.patch_flux.fert_to_sminn[0] = (0..npft).fold(0.0, |acc, m| acc + s.pft.fert_p[m]);
}

/// `CNSoyfix`：大豆共生固氮（`DEF_USE_CNSOYFIXN`）。
pub fn cn_soyfix(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, _sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut fxw: f64;
    let mut fxn: f64 = 0.0;
    let mut fxg: f64;
    let mut fxr: f64;
    let mut soy_ndemand: f64;
    let mut rwat: f64;
    let mut swat: f64;
    let mut rz: f64;
    let wf: f64;
    let sminnthreshold1: f64 = 30.0;
    let sminnthreshold2: f64 = 10.0;
    let gddfracthreshold1: f64 = 0.15;
    let gddfracthreshold2: f64 = 0.30;
    let gddfracthreshold3: f64 = 0.55;
    let gddfracthreshold4: f64 = 0.75;
    rwat = 0.0;
    swat = 0.0;
    rz = 0.0;
    for j in 0..d.nl_soil {
        if 0.5_f64.contract(p.dz_soi[j], p.z_soi[j]) <= 0.05 {
            rwat = ((-p.porsl[j]).contract(
                (316230.0 / (-p.psi0[j])).lpow(-(1.0 / p.bsw[j])),
                p.h2osoi[j],
            ))
            .contract(p.dz_soi[j], rwat);
            swat = ((-p.porsl[j]).contract(
                (316230.0 / (-p.psi0[j])).lpow(-(1.0 / p.bsw[j])),
                p.porsl[j],
            ))
            .contract(p.dz_soi[j], swat);
            rz += p.dz_soi[j];
        }
    }
    let tsw: f64 = rwat / rz;
    let stsw: f64 = swat / rz;
    if rz > 0.0 && stsw > 0.0 {
        wf = tsw / stsw;
    } else {
        wf = 0.0;
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        if s.pft.croplive_p[m] && (ivt == 23 || ivt == 24 || ivt == 77 || ivt == 78) {
            if s.patch.fpg[0] < 1.0 {
                soy_ndemand = (-s.pft_flux.plant_ndemand_p[m])
                    .contract(s.patch.fpg[0], s.pft_flux.plant_ndemand_p[m]);
                fxw = wf / 0.85;
                if s.patch.sminn[0] > sminnthreshold1 {
                    fxn = 0.0;
                } else if s.patch.sminn[0] > sminnthreshold2 && s.patch.sminn[0] <= sminnthreshold1
                {
                    fxn = (-0.005_f64).contract(s.patch.sminn[0] * 10.0, 1.5);
                } else if s.patch.sminn[0] <= sminnthreshold2 {
                    fxn = 1.0;
                }
                if s.pft.hui_p[m] <= gddfracthreshold1 {
                    fxg = 0.0;
                } else if s.pft.hui_p[m] > gddfracthreshold1 && s.pft.hui_p[m] <= gddfracthreshold2
                {
                    fxg = 6.67_f64.contract(s.pft.hui_p[m], -1.0);
                } else if s.pft.hui_p[m] > gddfracthreshold2 && s.pft.hui_p[m] <= gddfracthreshold3
                {
                    fxg = 1.0;
                } else if s.pft.hui_p[m] > gddfracthreshold3 && s.pft.hui_p[m] <= gddfracthreshold4
                {
                    fxg = (-5.0_f64).contract(s.pft.hui_p[m], 3.75);
                } else {
                    fxg = 0.0;
                }
                fxr = 0.0_f64.max(1.0_f64.min(fxw).min(fxn) * fxg);
                s.pft_flux.soyfixn_p[m] = (fxr * soy_ndemand).min(soy_ndemand);
            } else {
                s.pft_flux.soyfixn_p[m] = 0.0;
            }
        } else {
            s.pft_flux.soyfixn_p[m] = 0.0;
        }
    }
    s.patch_flux.soyfixn_to_sminn[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft_flux.soyfixn_p[m].contract(p.pftfrac[m], acc)
    });
}
