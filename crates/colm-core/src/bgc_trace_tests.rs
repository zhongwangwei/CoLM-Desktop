use super::*;
use crate::bgc_state::BgcDims;

/// 写出再读回：阶段名补齐到 32 字节后还原，物理输入与状态都逐位保留，
/// 整数/逻辑量经 f64 往返后回到原值。
#[test]
fn record_round_trips_through_the_fortran_layout() {
    let mut state = BgcState::new(2, BgcDims::default());
    state.pft.leafc_p = vec![1.5, -0.0];
    state.patch.altmax_lastyear_indx = vec![7];
    state.patch.skip_balance_check = vec![true];
    let mut bytes = Vec::new();
    write_record(&mut bytes, "CNMResp", &[("deltim", vec![1800.0])], &state).unwrap();
    let records = read_trace(&bytes).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tag, "CNMResp");
    assert_eq!(records[0].input("deltim"), Some(&[1800.0][..]));

    let mut back = BgcState::new(2, BgcDims::default());
    back.load_trace_fields(&records[0].state).unwrap();
    assert_eq!(back, state);
    assert_eq!(back.pft.leafc_p[1].to_bits(), (-0.0_f64).to_bits());
}

/// 超过 32 字节的阶段名与 Fortran `character(len=32)` 一样截断。
#[test]
fn long_tags_are_truncated_like_fortran() {
    let state = BgcState::new(1, BgcDims::default());
    let mut bytes = Vec::new();
    write_record(
        &mut bytes,
        "calc_plant_nutrient_demand_CLM45_default",
        &[],
        &state,
    )
    .unwrap();
    assert_eq!(
        read_trace(&bytes).unwrap()[0].tag,
        "calc_plant_nutrient_demand_CLM45"
    );
}
