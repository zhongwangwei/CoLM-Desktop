//! `MOD_BGC_CNBalanceCheck.F90`：步首记下 C/N 总量，步末检查收支。

use crate::bgc_state::BgcState;

/// `BeginCNBalance`：各加法按源码左结合（GIMPLE 未收缩）。
pub fn begin_cn_balance(s: &mut BgcState) {
    let p = &mut s.patch;
    p.col_begcb[0] = p.totcolc[0];
    p.col_begnb[0] = p.totcoln[0];
    p.col_vegbegcb[0] = p.totvegc[0] + p.ctrunc_veg[0];
    p.col_vegbegnb[0] = p.totvegn[0] + p.ntrunc_veg[0];
    p.col_soilbegcb[0] = p.totsomc[0] + p.totlitc[0] + p.totcwdc[0] + p.ctrunc_soil[0];
    p.col_soilbegnb[0] = p.totsomn[0] + p.totlitn[0] + p.totcwdn[0] + p.ntrunc_soil[0];
    p.col_sminnbegnb[0] = p.sminn[0];
}
