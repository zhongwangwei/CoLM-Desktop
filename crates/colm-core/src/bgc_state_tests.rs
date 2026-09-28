use super::*;

/// 新状态的初值与上游 `allocate_*` 一致：实数 `spval`、整数 `spval_i4`、逻辑 `.false.`，
/// 形状按 Fortran 列主序展平（`numpatch` 维去掉）。
#[test]
fn new_state_uses_the_upstream_fill_values_and_shapes() {
    let dims = BgcDims::default();
    let state = BgcState::new(3, dims);
    assert_eq!(state.pft.leafc_p, vec![crate::MISSING; 3]);
    assert_eq!(
        state.patch.decomp_cpools_vr.len(),
        dims.nl_soil_full * dims.ndecomp_pools
    );
    assert_eq!(state.f64_field("leafc_p").unwrap().len(), 3);
    assert_eq!(state.i32_field("altmax_lastyear_indx"), Some(&vec![-9999]));
    assert_eq!(state.bool_field("skip_balance_check"), Some(&vec![false]));
    assert!(state.f64_field("no_such_field").is_none());
}
