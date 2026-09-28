//! `MOD_BGC_CNAnnualUpdate.F90`：年末累计量的滚动。
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

use crate::bgc_driver::is_end_of_year;
use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;

/// `CNAnnualUpdate`：年末把 `tempsum_*`/`tempmax_*` 转成 `annsum_*`/`annmax_*`。
pub fn cn_annual_update(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, _sw: BgcSwitches) {
    let npft = p.pftclass.len();
    if is_end_of_year(p.idate, p.deltim) {
        for m in 0..npft {
            s.pft.annsum_potential_gpp_p[m] = s.pft.tempsum_potential_gpp_p[m];
            s.pft.tempsum_potential_gpp_p[m] = 0.0;
            s.pft.annmax_retransn_p[m] = s.pft.tempmax_retransn_p[m];
            s.pft.tempmax_retransn_p[m] = 0.0;
            s.pft.annavg_tref_p[m] = s.pft.tempavg_tref_p[m];
            s.pft.tempavg_tref_p[m] = 0.0;
            s.pft.annsum_npp_p[m] = s.pft.tempsum_npp_p[m] * p.deltim;
            s.pft.tempsum_npp_p[m] = 0.0;
        }
    }
}
