//! `MOD_BGC_Veg_CNNDynamics.F90`：生物固氮（作物施肥与大豆固氮在 `#ifdef CROP` 下，尚未移植）。

use crate::bgc_state::BgcState;
use crate::calendar::is_leap_year;
use crate::MISSING;

/// `CNNFixation`：`nfix = max(0, 1.8·(1 − exp(−0.003·npp·spy))/spy)`，`spy` = 一年秒数。
/// GIMPLE 把 `86400*dayspyr` 算一次，指数里是 `(npp·0.003)·spy`。
pub fn cn_n_fixation(s: &mut BgcState, idate: [i32; 3]) {
    let dayspyr = if is_leap_year(idate[0]) { 366.0 } else { 365.0 };
    let lag_npp = s.patch.lag_npp[0];
    s.patch_flux.nfix_to_sminn[0] = if lag_npp != MISSING {
        let seconds = dayspyr * 86400.0;
        let t = ((1.0 - (-(lag_npp * 0.003 * seconds)).exp()) * 1.8) / seconds;
        t.max(0.0)
    } else {
        0.0
    };
}
